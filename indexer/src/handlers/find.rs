//! The `/find` funnel's HTTP seam: `POST /find/next`, `POST /find/confirm`,
//! `GET /find/stats`. Everything below is transport — loading rows, shaping
//! JSON, validating input. All the engine maths lives in `crate::find`.
//!
//! Two things this layer is solely responsible for:
//!
//! **Leak control.** `/api/data/*` sells this catalog per request in NEB via
//! x402, so `/find` must never hand back anything that reconstructs it. The
//! candidate set, per-app facet vectors, scores for non-shortlisted apps and the
//! facet vocabulary itself all stay server-side; a turn discloses only the next
//! question and an integer `candidateCount` (A6/A20).
//!
//! **Anti-gaming.** Surfacing here drives traffic, which drives ad revenue, so
//! farming confirmations is directly profitable. The defence is a
//! bounded-influence blend, plus Turnstile and rate limiting — the blend's cap
//! lives in `find::blend`, and the Turnstile check and fixed-window rate-limit
//! gate live in the Next.js route (`app/src/app/api/find/confirm/route.ts`).
//! Only the rate limit is a gate: a failed Turnstile no longer refuses the
//! write, it records `turnstileVerified: false` and `store::load_learned`
//! declines to train on the row (A80). Farming still buys nothing, but the
//! rows Turnstile costs us are now countable instead of gone. It is
//! deliberately not described as "the standard anti-shilling defence" (A18): the
//! retrieved poisoning surveys recommend detection and robust training, and
//! overstating the pedigree would mislead the next reader.

use crate::api::{bad_request, ApiError, ApiState};
use crate::find::{blend, facets, params, scoring, selection, store, Answer, Candidate, FacetRef};
use crate::handlers::apps::{AppDto, TagDto};
use crate::handlers::engine::to_rfc3339;
use axum::extract::{Json, State};
use axum::routing::{get, post};
use axum::Router;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

/// Upper bound on a submitted answer history. The engine caps questions at
/// `params::MAX_QUESTIONS` (8) and the app's Zod schemas already reject longer
/// arrays (A41), so anything past this is a body that bypassed them — rejected
/// here rather than fed to a per-answer loop.
const MAX_ANSWERS: usize = 16;

// ---------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------

/// Every field defaults so an empty body `{}` is a valid first turn.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FindNextReq {
    #[serde(default)]
    answers: Vec<Answer>,
    #[serde(default)]
    force_results: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindQuestionDto {
    facet: FacetRef,
    prompt: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindShortlistEntry {
    app: AppDto,
    /// The normalized posterior for this app, not its blended score — the
    /// number the UI shows as "how sure are we", which is the answer-conditioned
    /// probability and nothing else.
    confidence: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindNextRes {
    /// `Some` iff `done` is false.
    question: Option<FindQuestionDto>,
    /// **Empty unless `done`.** This is a leak control, not a UI convenience
    /// (A6/A20): `/api/data/*` sells this catalog per request in NEB via x402,
    /// so returning ranked apps on every turn would let a caller sweep answer
    /// combinations and enumerate the database one HTTP call at a time.
    /// `candidateCount` carries the progress signal instead.
    shortlist: Vec<FindShortlistEntry>,
    candidate_count: usize,
    questions_asked: usize,
    done: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindConfirmReq {
    #[serde(default)]
    answers: Vec<Answer>,
    app_id: String,
    outcome: String,
    visitor_id: String,
    session_id: String,
    /// Whether the confirm that produced this outcome may be trained on — the
    /// Next.js route's `mayRecordOutcome(configured, verified)`, which is true
    /// when Turnstile is unconfigured (local/simulation) and when a real token
    /// passed. Stored, never gated on: A80 replaced A29's 403 with a recorded
    /// flag, because refusing the write dropped exactly the VPN, privacy-browser
    /// and slow-connection visitors and made the drop rate unmeasurable.
    ///
    /// Defaults to **false**, the opposite of the column's own default, and for
    /// the opposite reason: a body reaching this handler without the field is a
    /// caller that bypassed the route, and the safe reading of an unstated claim
    /// on an anti-farming flag is "unproven".
    #[serde(default)]
    turnstile_verified: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindStatsRes {
    avg_questions_to_confirm: Option<f64>,
}

// ---------------------------------------------------------------------
// Response shaping (pure — no pool, no async, unit-tested below)
// ---------------------------------------------------------------------

/// Descending order over blended scores, with every non-finite score forced to
/// the back.
///
/// The guard lives here rather than in `blend` because this is the first and
/// only place a score is ordered, and `blend::blend` deliberately does not
/// sanitize its `content` argument (A52) — clamping a NaN posterior there would
/// bury a scorer bug under a plausible-looking score. A NaN compares false
/// against everything, so `partial_cmp(..).unwrap_or(Equal)` would silently
/// scramble the shortlist; `total_cmp` plus an explicit finite-first split
/// cannot. Non-finite entries are pushed back rather than dropped, so a corrupt
/// score can never win and can never empty the shortlist either.
fn score_order(a: f64, b: f64) -> Ordering {
    match (a.is_finite(), b.is_finite()) {
        (true, true) => b.total_cmp(&a),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        // Equal under a stable sort, so corrupt entries keep input order.
        (false, false) => Ordering::Equal,
    }
}

/// The top `limit` candidates as `(app id, confidence)`, ranked by
/// `blend::blend` descending. `confidence` is the normalized posterior, which is
/// the content term of that blend (A32).
fn rank_shortlist<'a>(
    candidates: &'a [Candidate],
    post: &[f64],
    limit: usize,
) -> Vec<(&'a str, f64)> {
    let mut ranked: Vec<(&str, f64, f64)> = candidates
        .iter()
        .zip(post)
        .map(|(c, &p)| {
            (
                c.app_id.as_str(),
                blend::blend(p, c.support, c.outcome_mean),
                p,
            )
        })
        .collect();
    ranked.sort_by(|a, b| score_order(a.1, b.1));
    ranked
        .into_iter()
        .take(limit)
        .map(|(app_id, _, confidence)| (app_id, confidence))
        .collect()
}

/// The whole `/find/next` decision, factored out of the handler so it is
/// testable with no database: what to disclose, and when.
///
/// The shortlist resolves candidates back to `apps` by id rather than by
/// position, so it does not silently depend on `facets::candidates_from_apps`
/// preserving input order.
///
/// `question` is only read when `!done`, and `selection::should_stop` returns
/// `NoInformativeQuestion` whenever it is `None`, so `!done` implies `Some`.
fn next_response(
    apps: &[AppDto],
    candidates: &[Candidate],
    post: &[f64],
    question: Option<FacetRef>,
    questions_asked: usize,
    done: bool,
) -> FindNextRes {
    let shortlist = if done {
        let by_id: HashMap<&str, &AppDto> = apps.iter().map(|app| (app.id.as_str(), app)).collect();
        rank_shortlist(candidates, post, params::SHORTLIST_LIMIT)
            .into_iter()
            .filter_map(|(app_id, confidence)| {
                by_id.get(app_id).map(|app| FindShortlistEntry {
                    app: (*app).clone(),
                    confidence,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    FindNextRes {
        question: if done {
            None
        } else {
            question.map(|facet| FindQuestionDto {
                prompt: facets::prompt_for(&facet),
                facet,
            })
        },
        shortlist,
        candidate_count: candidates.len(),
        questions_asked,
        done,
    }
}

/// Whether this turn ends the funnel: the engine's own stopping rule, or the
/// visitor pressing "show me results". The second half is brief criterion 1's
/// "can always bail to results early", so it is a guard, not a convenience.
fn is_done(stop: Option<selection::StopReason>, force_results: bool) -> bool {
    stop.is_some() || force_results
}

/// The `MAX_ANSWERS` bound, shared by both POST routes so one test covers both
/// call sites rather than only the one that happens to be reachable.
fn check_answer_bound(count: usize) -> Result<(), ApiError> {
    if count > MAX_ANSWERS {
        return Err(bad_request(format!("at most {MAX_ANSWERS} answers")));
    }
    Ok(())
}

/// The confirm response, pinned in one place so A20's disclosure rule is a
/// value a test can read rather than a literal buried in an async handler. It
/// is `{"ok": true}` whether the row was inserted, replayed, or recorded
/// unverified — a response that varied would tell a farmer which it was.
fn confirm_response() -> serde_json::Value {
    serde_json::json!({ "ok": true })
}

/// Validates a confirm body, returning the `questionsAsked` count to store.
///
/// `turnstile_verified` is deliberately absent from these checks: it changes
/// what the row is worth, not whether it is well-formed (A80).
fn validate_confirm(req: &FindConfirmReq) -> Result<i32, ApiError> {
    check_answer_bound(req.answers.len())?;
    // An empty sessionId would collapse every visitor onto one idempotency key,
    // so the first confirm of a session would silently suppress everyone else's.
    if req.app_id.is_empty() || req.visitor_id.is_empty() || req.session_id.is_empty() {
        return Err(bad_request("appId, visitorId and sessionId are required"));
    }
    if blend::outcome_weight(&req.outcome).is_none() {
        return Err(bad_request(format!("unknown outcome: {}", req.outcome)));
    }
    Ok(req.answers.len() as i32)
}

// ---------------------------------------------------------------------
// Catalog loading
// ---------------------------------------------------------------------

/// `handlers::apps`' own row type and `to_dto` are private, and its
/// `fetch_all_approved` issues one tag query per app — affordable there, not
/// here, where the whole catalog loads on every turn of every session.
struct AppRow {
    id: String,
    slug: String,
    name: String,
    tagline: String,
    description: String,
    url: String,
    icon_url: Option<String>,
    category: String,
    chain: String,
    status: String,
    created_at: NaiveDateTime,
    submitted_by: Option<String>,
    vote_count: i32,
    vote_weight: f64,
    stake_total: f64,
    view_count: i32,
    rank_score: f64,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for AppRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(AppRow {
            id: row.try_get("id")?,
            slug: row.try_get("slug")?,
            name: row.try_get("name")?,
            tagline: row.try_get("tagline")?,
            description: row.try_get("description")?,
            url: row.try_get("url")?,
            icon_url: row.try_get("iconUrl")?,
            category: row.try_get("category")?,
            chain: row.try_get("chain")?,
            status: row.try_get("status")?,
            created_at: row.try_get("createdAt")?,
            submitted_by: row.try_get("submittedBy")?,
            vote_count: row.try_get("voteCount")?,
            vote_weight: row.try_get("voteWeight")?,
            stake_total: row.try_get("stakeTotal")?,
            view_count: row.try_get("viewCount")?,
            rank_score: row.try_get("rankScore")?,
        })
    }
}

/// Every approved app with its tags, in two statements rather than 1 + N.
///
/// Column spellings mirror `handlers::apps::APP_ROW_COLUMNS` exactly; the tag
/// join mirrors its `fetch_tags`, including the stake-descending order, so an
/// `AppDto` from here is indistinguishable from one served by `/apps/*`.
async fn fetch_approved_apps(pool: &PgPool) -> Result<Vec<AppDto>, ApiError> {
    let rows: Vec<AppRow> = sqlx::query_as(
        r#"
        SELECT id, slug, name, tagline, description, url, "iconUrl", category, chain,
               status, "createdAt", "submittedBy", "voteCount", "voteWeight",
               "stakeTotal", "viewCount", "rankScore"
        FROM "App" WHERE status = 'approved'
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(crate::api::internal)?;

    type TagRow = (String, String, String, String, String, f64, Option<String>);
    let tag_rows: Vec<TagRow> = sqlx::query_as(
        r#"
        SELECT at."appId", at.id, at."tagId", t.slug, t.name, at."stakeTotal", at."suggestedBy"
        FROM "AppTag" at
        JOIN "Tag" t ON t.id = at."tagId"
        ORDER BY at."stakeTotal" DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(crate::api::internal)?;

    let mut tags_by_app: HashMap<String, Vec<TagDto>> = HashMap::new();
    for (app_id, id, tag_id, slug, name, stake_total, suggested_by) in tag_rows {
        tags_by_app.entry(app_id).or_default().push(TagDto {
            id,
            tag_id,
            slug,
            name,
            stake_total,
            suggested_by,
        });
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let tags = tags_by_app.remove(&row.id).unwrap_or_default();
            AppDto {
                id: row.id,
                slug: row.slug,
                name: row.name,
                tagline: row.tagline,
                description: row.description,
                url: row.url,
                icon_url: row.icon_url,
                category: row.category,
                chain: row.chain,
                status: row.status,
                created_at: to_rfc3339(row.created_at),
                submitted_by: row.submitted_by,
                vote_count: row.vote_count,
                vote_weight: row.vote_weight,
                stake_total: row.stake_total,
                view_count: row.view_count,
                rank_score: row.rank_score,
                tags,
                trend: None,
            }
        })
        .collect())
}

// ---------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------

async fn next(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<FindNextReq>,
) -> Result<Json<FindNextRes>, ApiError> {
    check_answer_bound(req.answers.len())?;

    let apps = fetch_approved_apps(&state.pool).await?;
    let learned = store::load_learned(&state.pool).await?;
    let candidates = facets::candidates_from_apps(&apps, &learned);

    let post = scoring::posterior(&candidates, &req.answers);
    let facet_pool = facets::facet_pool(&candidates);
    let question = selection::select_question(&candidates, &req.answers, &facet_pool);
    let stop = selection::should_stop(&post, req.answers.len(), question.as_ref());
    let done = is_done(stop, req.force_results);

    let res = next_response(&apps, &candidates, &post, question, req.answers.len(), done);

    // The evidence `params::MIN_CONTENT_GAP`'s doc comment asks for before
    // anyone retunes it: the posterior gap actually measured at the stopping
    // point, and whether it is wide enough that no volume of farmed support
    // could flip the pair (A16). Debug level — one line per completed session.
    if let [top, second, ..] = res.shortlist.as_slice() {
        log::debug!(
            "find: top-two posterior gap {:.4} (MIN_CONTENT_GAP {:.4}, farming-proof {})",
            top.confidence - second.confidence,
            params::MIN_CONTENT_GAP,
            blend::preserves_order(top.confidence, second.confidence),
        );
    }

    Ok(Json(res))
}

async fn confirm(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<FindConfirmReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let questions_asked = validate_confirm(&req)?;

    let inserted = store::record_outcome(
        &state.pool,
        &req.visitor_id,
        &req.session_id,
        &req.app_id,
        &req.outcome,
        &req.answers,
        questions_asked,
        req.turnstile_verified,
    )
    .await?;

    // Only on a first write: a replay must not inflate the facet tallies. The
    // response is identical either way, so it never discloses which it was.
    if inserted {
        store::bump_facet_stats(&state.pool, &req.answers).await?;
    }

    Ok(Json(confirm_response()))
}

/// Brief success criterion 7 — average questions-to-confirm, so the self-tuning
/// loop's effect is observable rather than assumed. One aggregate number over
/// sessions; it discloses nothing about the catalog.
async fn stats(State(state): State<Arc<ApiState>>) -> Result<Json<FindStatsRes>, ApiError> {
    Ok(Json(FindStatsRes {
        avg_questions_to_confirm: store::avg_questions_to_confirm(&state.pool).await?,
    }))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new()
        .route("/find/next", post(next))
        .route("/find/confirm", post(confirm))
        .route("/find/stats", get(stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::{AnswerValue, FacetKind};
    use std::collections::HashSet;

    fn app(id: &str) -> AppDto {
        AppDto {
            id: id.to_string(),
            slug: format!("slug-{id}"),
            name: format!("Name {id}"),
            tagline: String::new(),
            description: String::new(),
            url: "https://example.com".into(),
            icon_url: None,
            category: "defi".into(),
            chain: "solana".into(),
            status: "approved".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            submitted_by: None,
            vote_count: 0,
            vote_weight: 0.0,
            stake_total: 0.0,
            view_count: 0,
            rank_score: 0.0,
            tags: Vec::new(),
            trend: None,
        }
    }

    fn candidate(id: &str, content_score: f64, support: f64, outcome_mean: f64) -> Candidate {
        Candidate {
            app_id: id.to_string(),
            category: "defi".into(),
            chain: "solana".into(),
            tags: HashSet::new(),
            content_score,
            support,
            outcome_mean,
        }
    }

    fn facet(value: &str) -> FacetRef {
        FacetRef {
            kind: FacetKind::Category,
            value: value.to_string(),
        }
    }

    fn ids(res: &FindNextRes) -> Vec<&str> {
        res.shortlist.iter().map(|e| e.app.id.as_str()).collect()
    }

    #[test]
    fn mid_funnel_returns_a_question_and_no_shortlist() {
        let apps = vec![app("a0"), app("a1")];
        let candidates = vec![
            candidate("a0", 0.5, 0.0, 0.0),
            candidate("a1", 0.9, 0.0, 0.0),
        ];
        let post = scoring::posterior(&candidates, &[]);

        let res = next_response(&apps, &candidates, &post, Some(facet("defi")), 1, false);
        let json = serde_json::to_string(&res).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(json.contains(r#""shortlist":[]"#), "{json}");
        assert!(value["question"]["facet"].is_object(), "{json}");
        assert!(
            !value["question"]["prompt"].as_str().unwrap().is_empty(),
            "{json}"
        );
        assert_eq!(value["candidateCount"], 2);
        assert_eq!(value["questionsAsked"], 1);
        assert_eq!(value["done"], false);
    }

    #[test]
    fn done_serializes_question_as_null() {
        let apps = vec![app("a0")];
        let candidates = vec![candidate("a0", 0.5, 0.0, 0.0)];
        let post = scoring::posterior(&candidates, &[]);

        // A question is supplied and must still be withheld: `done` decides.
        let res = next_response(&apps, &candidates, &post, Some(facet("defi")), 3, true);
        let json = serde_json::to_string(&res).unwrap();

        assert!(json.contains(r#""question":null"#), "{json}");
    }

    /// The shortlist is ordered by `blend::blend`, not by the posterior. Every
    /// candidate here has the same content score, so the posterior is uniform
    /// and the learned term alone decides the order — which is the reverse of
    /// the input order, so a sort on the posterior (stable, hence input order)
    /// fails this.
    #[test]
    fn shortlist_is_capped_and_ordered_by_the_blended_score() {
        let apps: Vec<AppDto> = (0..7).map(|i| app(&format!("a{i}"))).collect();
        let candidates: Vec<Candidate> = (0..7)
            .map(|i| candidate(&format!("a{i}"), 0.5, i as f64 * 30.0, 1.0))
            .collect();
        let post = scoring::posterior(&candidates, &[]);

        let res = next_response(&apps, &candidates, &post, None, 4, true);

        // The literal 5 is A20's leak cap, restated here as observable
        // behaviour: `assert_eq!(len, params::SHORTLIST_LIMIT)` would scale with
        // the constant and so could not notice it drifting.
        assert_eq!(res.shortlist.len(), 5);
        assert_eq!(ids(&res), ["a6", "a5", "a4", "a3", "a2"]);
        // `confidence` is the posterior, not the blended score: uniform here.
        for entry in &res.shortlist {
            let confidence = entry.confidence;
            assert!((confidence - 1.0 / 7.0).abs() < 1e-9, "{confidence}");
        }
    }

    /// Brief success criterion 2: no answer path can empty the shortlist. The
    /// `[EPS, 1-EPS]` clamp in `scoring` guarantees no candidate is eliminated,
    /// and nothing here may filter one out afterwards.
    #[test]
    fn all_skip_path_still_yields_a_full_shortlist() {
        let apps: Vec<AppDto> = (0..3).map(|i| app(&format!("a{i}"))).collect();
        let candidates: Vec<Candidate> = (0..3)
            .map(|i| candidate(&format!("a{i}"), 0.1 * i as f64, 0.0, 0.0))
            .collect();
        let answers: Vec<Answer> = ["defi", "nft", "gaming"]
            .iter()
            .map(|value| Answer {
                facet: facet(value),
                value: AnswerValue::Skip,
            })
            .collect();
        let post = scoring::posterior(&candidates, &answers);

        let res = next_response(&apps, &candidates, &post, None, answers.len(), true);

        assert_eq!(res.shortlist.len(), 3);
        for entry in &res.shortlist {
            let confidence = entry.confidence;
            assert!(confidence.is_finite() && confidence > 0.0, "{confidence}");
        }
    }

    /// **Leak control (A6/A20, brief success criterion 3).** A mid-funnel turn
    /// over the whole catalog must disclose no app identity and no facet vector
    /// — only the next question and an integer count. This test exists so a
    /// later change cannot quietly start returning identities on every turn,
    /// which would let a caller sweep answer combinations and enumerate the
    /// catalog `/api/data/*` sells per request in NEB.
    #[test]
    fn mid_funnel_response_leaks_no_identities_a6_a20() {
        let apps: Vec<AppDto> = (0..100).map(|i| app(&format!("app-{i:03}"))).collect();
        let candidates: Vec<Candidate> = (0..100)
            .map(|i| candidate(&format!("app-{i:03}"), i as f64 / 100.0, 0.0, 0.0))
            .collect();
        let post = scoring::posterior(&candidates, &[]);

        let res = next_response(&apps, &candidates, &post, Some(facet("defi")), 2, false);
        let json = serde_json::to_string(&res).unwrap();

        for app in &apps {
            assert!(!json.contains(&app.id), "app id leaked: {json}");
            assert!(!json.contains(&app.slug), "app slug leaked: {json}");
            assert!(!json.contains(&app.name), "app name leaked: {json}");
        }
        // Every field only an `AppDto` carries, plus the two keys a shortlist
        // entry would add. The question's own facet kind ("category") is
        // deliberately not on this list: one facet per turn is the disclosure
        // the funnel exists to make.
        for forbidden in [
            "facets",
            "tags",
            "rankScore",
            "voteWeight",
            "stakeTotal",
            "iconUrl",
            "submittedBy",
            "confidence",
            "\"app\"",
        ] {
            assert!(!json.contains(forbidden), "{forbidden} leaked: {json}");
        }

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "candidateCount",
                "done",
                "question",
                "questionsAsked",
                "shortlist"
            ]
        );
        assert_eq!(value["candidateCount"], 100);
    }

    /// The comparator's four branches, pinned directly. Going through
    /// `sort_by` cannot pin the `(non-finite, non-finite)` case: making that
    /// branch return `Less` or `Greater` yields an inconsistent comparator,
    /// whose effect on a short slice is unspecified, so the sort happens to
    /// come out identical. `Equal` is what makes a stable sort leave corrupt
    /// entries in input order instead of inventing one.
    #[test]
    fn score_order_pins_all_four_branches() {
        assert_eq!(score_order(0.5, 0.2), Ordering::Less);
        assert_eq!(score_order(0.2, 0.5), Ordering::Greater);
        assert_eq!(score_order(0.5, 0.5), Ordering::Equal);
        assert_eq!(score_order(0.5, f64::NAN), Ordering::Less);
        assert_eq!(score_order(f64::NAN, 0.5), Ordering::Greater);
        assert_eq!(score_order(f64::NAN, f64::INFINITY), Ordering::Equal);
        assert_eq!(score_order(f64::INFINITY, f64::NAN), Ordering::Equal);
    }

    /// A52: this is the only place a score is ordered, so it owns the
    /// non-finite guard. A NaN compares false against everything — with
    /// `partial_cmp(..).unwrap_or(Equal)` it would scramble the order instead
    /// of failing loudly.
    #[test]
    fn non_finite_scores_sort_last_and_never_scramble_a52() {
        let apps: Vec<AppDto> = (0..4).map(|i| app(&format!("a{i}"))).collect();
        let candidates: Vec<Candidate> = (0..4)
            .map(|i| candidate(&format!("a{i}"), 0.5, 0.0, 0.0))
            .collect();
        let post = vec![0.2, f64::NAN, 0.5, f64::INFINITY];

        let res = next_response(&apps, &candidates, &post, None, 3, true);

        // Finite scores first, correctly ordered; corrupt ones keep input order
        // behind them, and are dropped from neither the list nor the count.
        assert_eq!(ids(&res), ["a2", "a0", "a1", "a3"]);
    }

    #[test]
    fn empty_body_is_a_valid_first_turn() {
        let req: FindNextReq = serde_json::from_str("{}").unwrap();
        assert!(req.answers.is_empty());
        assert!(!req.force_results);
    }

    #[test]
    fn confirm_body_parses_camel_case() {
        let req: FindConfirmReq = serde_json::from_str(
            r#"{"answers":[{"facet":{"kind":"tag","value":"lending"},"value":"yes"}],
                "appId":"a0","outcome":"confirmed","visitorId":"v0","sessionId":"s0",
                "turnstileVerified":true}"#,
        )
        .unwrap();
        assert!(req.turnstile_verified);
        // `ApiError` is not `Debug`, so no `unwrap` on these Results.
        match validate_confirm(&req) {
            Ok(questions_asked) => assert_eq!(questions_asked, 1),
            Err(_) => panic!("a well-formed confirm body must validate"),
        }
    }

    #[test]
    fn confirm_rejects_unknown_outcomes_and_overlong_histories() {
        let base = |outcome: &str, answers: Vec<Answer>| FindConfirmReq {
            answers,
            app_id: "a0".into(),
            outcome: outcome.into(),
            visitor_id: "v0".into(),
            session_id: "s0".into(),
            turnstile_verified: true,
        };
        let answer = Answer {
            facet: facet("defi"),
            value: AnswerValue::Yes,
        };

        for outcome in ["confirmed", "clicked", "rejected"] {
            assert!(
                validate_confirm(&base(outcome, vec![])).is_ok(),
                "{outcome}"
            );
        }
        for bad in ["", "Confirmed", "upvoted"] {
            let Err(err) = validate_confirm(&base(bad, vec![])) else {
                panic!("outcome {bad:?} must be rejected");
            };
            assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST, "{bad}");
        }

        let overlong = vec![answer; 17];
        let Err(err) = validate_confirm(&base("confirmed", overlong)) else {
            panic!("an over-long answer history must be rejected");
        };
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
    }

    /// A31's idempotency key is `(sessionId, appId, outcome)` — no `visitorId`
    /// — so a blank `sessionId` would collapse every visitor onto one key and
    /// the first confirm would silently suppress everyone else's.
    #[test]
    fn confirm_rejects_an_empty_identity_field() {
        let req = |app_id: &str, visitor_id: &str, session_id: &str| FindConfirmReq {
            answers: Vec::new(),
            app_id: app_id.into(),
            outcome: "confirmed".into(),
            visitor_id: visitor_id.into(),
            session_id: session_id.into(),
            turnstile_verified: true,
        };

        assert!(validate_confirm(&req("a0", "v0", "s0")).is_ok());
        for (label, blank) in [
            ("appId", req("", "v0", "s0")),
            ("visitorId", req("a0", "", "s0")),
            ("sessionId", req("a0", "v0", "")),
        ] {
            let Err(err) = validate_confirm(&blank) else {
                panic!("an empty {label} must be rejected");
            };
            assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST, "{label}");
        }
    }

    /// A80: an unverified outcome is *recorded*, not refused, so validation must
    /// treat `turnstileVerified: false` as a perfectly well-formed body. A
    /// rejection added here would restore A29's dropped-row bias one layer down
    /// from where the user removed it.
    #[test]
    fn confirm_accepts_an_unverified_outcome_a78() {
        let req = |turnstile_verified: bool| FindConfirmReq {
            answers: Vec::new(),
            app_id: "a0".into(),
            outcome: "confirmed".into(),
            visitor_id: "v0".into(),
            session_id: "s0".into(),
            turnstile_verified,
        };

        for verified in [true, false] {
            assert!(validate_confirm(&req(verified)).is_ok(), "{verified}");
        }
    }

    /// The wire spelling and the default, which the route on the other side
    /// depends on. A body that omits the field is a caller that bypassed the
    /// route, and must read as *unproven* — the opposite of the column's own
    /// default, which only a Turnstile-unaware writer can reach.
    #[test]
    fn confirm_defaults_an_absent_turnstile_flag_to_false_a78() {
        let body = |extra: &str| {
            format!(
                r#"{{"appId":"a0","outcome":"confirmed","visitorId":"v0","sessionId":"s0"{extra}}}"#
            )
        };

        let absent: FindConfirmReq = serde_json::from_str(&body("")).unwrap();
        assert!(!absent.turnstile_verified);

        let present: FindConfirmReq =
            serde_json::from_str(&body(r#","turnstileVerified":true"#)).unwrap();
        assert!(present.turnstile_verified);
    }

    /// A20 for the confirm route: the response is `{"ok":true}` and nothing
    /// else, on every path. A field that varied with `inserted` or with the
    /// Turnstile flag would tell a farmer which of the two happened, which is
    /// the disclosure the constant response exists to withhold.
    #[test]
    fn confirm_response_discloses_nothing_a20() {
        assert_eq!(
            serde_json::to_string(&confirm_response()).unwrap(),
            r#"{"ok":true}"#
        );
    }

    /// The bound's **value** is contract, not just its existence: A41 caps both
    /// app-side paths at 16 and the indexer bound is defence in depth behind
    /// that same number, so a drift here would silently widen it. Both POST
    /// routes go through this one function, so this covers `/find/next` too.
    #[test]
    fn answer_history_is_bounded_at_sixteen_on_both_routes() {
        assert_eq!(MAX_ANSWERS, 16);
        assert!(check_answer_bound(0).is_ok());
        assert!(check_answer_bound(16).is_ok());
        let Err(err) = check_answer_bound(17) else {
            panic!("17 answers must be rejected");
        };
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
    }

    /// The final-turn half of the wire contract node 1.7 codes against
    /// (`FindShortlistEntry` in app/src/lib/types.ts): `app` is a full AppDto
    /// and `confidence` sits beside it, both in camelCase.
    #[test]
    fn shortlist_entries_carry_app_and_confidence() {
        let apps = vec![app("a0")];
        let candidates = vec![candidate("a0", 0.5, 0.0, 0.0)];
        let post = scoring::posterior(&candidates, &[]);

        let res = next_response(&apps, &candidates, &post, None, 5, true);
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&res).unwrap()).unwrap();

        let entry = &value["shortlist"][0];
        assert_eq!(entry["app"]["id"], "a0");
        assert_eq!(entry["app"]["iconUrl"], serde_json::Value::Null);
        assert_eq!(entry["confidence"], 1.0);
    }

    /// The shortlist names apps by id, not by position (A69), so it survives
    /// `apps` and `candidates` arriving in different orders. Nothing else in
    /// this file would notice if that silently became positional.
    #[test]
    fn shortlist_resolves_apps_by_id_not_by_position() {
        // Reversed relative to `candidates` below.
        let apps: Vec<AppDto> = (0..3).rev().map(|i| app(&format!("a{i}"))).collect();
        let candidates: Vec<Candidate> = (0..3)
            .map(|i| candidate(&format!("a{i}"), 0.5, i as f64 * 50.0, 1.0))
            .collect();
        let post = scoring::posterior(&candidates, &[]);

        let res = next_response(&apps, &candidates, &post, None, 3, true);

        // Ranked by the learned term, which ascends with i.
        assert_eq!(ids(&res), ["a2", "a1", "a0"]);
    }

    /// Brief criterion 1: the visitor can always bail to results early, so
    /// `forceResults` ends the funnel even when the engine wants to keep asking.
    #[test]
    fn force_results_ends_the_funnel_without_the_engine_stopping() {
        assert!(!is_done(None, false));
        assert!(is_done(None, true));
        assert!(is_done(Some(selection::StopReason::QuestionCap), false));
        assert!(is_done(
            Some(selection::StopReason::PosteriorThreshold),
            true
        ));
    }

    /// A68's route is new wire surface, so its one field name is pinned too.
    #[test]
    fn stats_response_spelling() {
        let json = serde_json::to_string(&FindStatsRes {
            avg_questions_to_confirm: Some(4.5),
        })
        .unwrap();
        assert_eq!(json, r#"{"avgQuestionsToConfirm":4.5}"#);
        let empty = serde_json::to_string(&FindStatsRes {
            avg_questions_to_confirm: None,
        })
        .unwrap();
        assert_eq!(empty, r#"{"avgQuestionsToConfirm":null}"#);
    }
}
