//! Ports every remaining `App`/`AppTag`/`Tag`-reading Prisma call site:
//! `app/src/lib/search.ts` (+ `fuzzy.ts`, the hardest piece — see its
//! module doc), `app/src/lib/queries.ts`'s `getAppDetail`, and the
//! `apps/[slug]`, `apps/related`, `apps/by-id/[appId]` routes. Ported
//! verbatim, not redesigned: the DB pre-filter stays a loose `ILIKE`, and
//! precise relevance/fuzzy/facet/pagination logic stays in-process over the
//! filtered row set, exactly like the TS original — see search.ts's own doc
//! comment on why (Postgres FTS would be a real behavior change, not a pure
//! port, and wasn't asked for).

use crate::api::{not_found, ApiError, ApiState};
use axum::extract::{Json, Path, Query, State};
use axum::routing::{get, patch, post};
use axum::Router;
use chrono::NaiveDateTime;
use crate::handlers::engine::to_rfc3339;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
    pub id: String,
    pub tag_id: String,
    pub slug: String,
    pub name: String,
    pub stake_total: f64,
    pub suggested_by: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppDto {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub tagline: String,
    pub description: String,
    pub url: String,
    pub icon_url: Option<String>,
    pub category: String,
    pub chain: String,
    pub status: String,
    pub created_at: String,
    pub submitted_by: Option<String>,
    pub vote_count: i32,
    pub vote_weight: f64,
    pub stake_total: f64,
    pub view_count: i32,
    pub rank_score: f64,
    pub tags: Vec<TagDto>,
    /// Only populated by `search_apps` (the Discover list) — every other
    /// caller of `to_dto` (app detail, related apps, ...) has no reason to
    /// pay for the extra `AppStatsSnapshot` query, so it defaults to `None`
    /// there instead of always being fetched.
    pub trend: Option<TrendDto>,
}

/// How much each of an app's stats moved over `interval_days`, as a percent
/// change from its `AppStatsSnapshot` baseline — the data behind Discover's
/// AppCard subtext ("+12%/7d"). Always the weekly window (see
/// `TREND_DISPLAY_DAYS`) regardless of the active sort — "trending_month"
/// reorders results using its own separate 30-day baseline, but every
/// card's displayed delta stays weekly. Each field is independently `None`
/// (rather than the whole struct) when its own baseline value was exactly 0
/// — "grew from 0" has no meaningful percent, and returning `None` there
/// instead of `Infinity`/a huge number keeps one zero-baseline stat from
/// hiding the other three, which usually still have a real baseline.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrendDto {
    pub interval_days: i64,
    pub vote_weight_pct: Option<f64>,
    pub stake_total_pct: Option<f64>,
    pub view_count_pct: Option<f64>,
    pub rank_score_pct: Option<f64>,
}

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

/// Fetches this app's `AppTag`+`Tag` rows, sorted by stake descending —
/// same as `serializeApp`'s `.sort((a, b) => b.stakeTotal - a.stakeTotal)`.
async fn fetch_tags(pool: &PgPool, app_id: &str) -> Result<Vec<TagDto>, ApiError> {
    let rows: Vec<(String, String, String, String, f64, Option<String>)> = sqlx::query_as(
        r#"
        SELECT at.id, at."tagId", t.slug, t.name, at."stakeTotal", at."suggestedBy"
        FROM "AppTag" at
        JOIN "Tag" t ON t.id = at."tagId"
        WHERE at."appId" = $1
        ORDER BY at."stakeTotal" DESC
        "#,
    )
    .bind(app_id)
    .fetch_all(pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(rows
        .into_iter()
        .map(|(id, tag_id, slug, name, stake_total, suggested_by)| TagDto {
            id,
            tag_id,
            slug,
            name,
            stake_total,
            suggested_by,
        })
        .collect())
}

fn to_dto(row: AppRow, tags: Vec<TagDto>) -> AppDto {
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
}

const APP_ROW_COLUMNS: &str = r#"id, slug, name, tagline, description, url, "iconUrl", category, chain, status, "createdAt", "submittedBy", "voteCount", "voteWeight", "stakeTotal", "viewCount", "rankScore""#;

async fn fetch_app_by(pool: &PgPool, column: &str, value: &str) -> Result<Option<AppDto>, ApiError> {
    let query = format!(r#"SELECT {APP_ROW_COLUMNS} FROM "App" WHERE {column} = $1"#);
    let row: Option<AppRow> = sqlx::query_as(&query)
        .bind(value)
        .fetch_optional(pool)
        .await
        .map_err(crate::api::internal)?;
    match row {
        None => Ok(None),
        Some(row) => {
            let tags = fetch_tags(pool, &row.id).await?;
            Ok(Some(to_dto(row, tags)))
        }
    }
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

/// All approved apps, with tags — the bulk fetch every route below filters/
/// scores in-process, same shape as `search.ts`'s loose-DB-filter-then-JS-
/// score approach (see this module's doc comment).
async fn fetch_all_approved(pool: &PgPool) -> Result<Vec<AppDto>, ApiError> {
    let query = format!(r#"SELECT {APP_ROW_COLUMNS} FROM "App" WHERE status = 'approved'"#);
    let rows: Vec<AppRow> = sqlx::query_as(&query)
        .fetch_all(pool)
        .await
        .map_err(crate::api::internal)?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let tags = fetch_tags(pool, &row.id).await?;
        out.push(to_dto(row, tags));
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// GET /apps/by-id/:id, GET /apps/by-slug/:slug, GET /apps/related
// ---------------------------------------------------------------------

async fn get_by_id(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<AppDto>, ApiError> {
    let app = fetch_app_by(&state.pool, "id", &id)
        .await?
        .ok_or_else(|| not_found("app not found"))?;
    Ok(Json(app))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDetailDto {
    pub app: AppDto,
    pub recent_votes: Vec<RecentVoteDto>,
    pub top_stakers: Vec<TopStakerDto>,
    pub views_last_7d: i64,
    pub snapshots: Vec<SnapshotPointDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentVoteDto {
    pub id: String,
    pub amount: f64,
    pub created_at: String,
    pub wallet: String,
    pub tx_sig: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopStakerDto {
    pub wallet: String,
    pub amount: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPointDto {
    pub date: String,
    pub vote_weight: f64,
    pub stake_total: f64,
    pub view_count: i32,
    pub rank_score: f64,
}

/// Same shape as `queries.ts`'s `getAppDetail` + `apps/[slug]/route.ts`
/// (previously two near-duplicate implementations of the same query — this
/// merges them into one, adding `queries.ts`'s `rankScore` field to the
/// snapshot points since nothing depended on its absence).
async fn get_by_slug(
    State(state): State<Arc<ApiState>>,
    Path(slug): Path<String>,
) -> Result<Json<AppDetailDto>, ApiError> {
    let app = fetch_app_by(&state.pool, "slug", &slug)
        .await?
        .ok_or_else(|| not_found("app not found"))?;

    type VoteRow = (String, f64, NaiveDateTime, Option<String>, Option<String>, Option<String>);
    let vote_rows: Vec<VoteRow> = sqlx::query_as(
            r#"
            SELECT v.id, v.amount, v."createdAt", v."txSig", u.wallet, u.handle
            FROM "Vote" v JOIN "User" u ON u.id = v."userId"
            WHERE v."appId" = $1
            ORDER BY v."createdAt" DESC
            LIMIT 10
            "#,
        )
        .bind(&app.id)
        .fetch_all(&state.pool)
        .await
        .map_err(crate::api::internal)?;
    let recent_votes = vote_rows
        .into_iter()
        .map(|(id, amount, created_at, tx_sig, wallet, handle)| RecentVoteDto {
            id,
            amount,
            created_at: to_rfc3339(created_at),
            wallet: handle.or(wallet).unwrap_or_default(),
            tx_sig,
        })
        .collect();

    let staker_rows: Vec<(String, f64, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT s."userId", SUM(s.amount) as total, u.wallet, u.handle
        FROM "Stake" s
        JOIN "AppTag" at ON at.id = s."appTagId"
        JOIN "User" u ON u.id = s."userId"
        WHERE at."appId" = $1 AND s.active = true
        GROUP BY s."userId", u.wallet, u.handle
        ORDER BY total DESC
        LIMIT 10
        "#,
    )
    .bind(&app.id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;
    let top_stakers = staker_rows
        .into_iter()
        .map(|(_, amount, wallet, handle)| TopStakerDto { wallet: handle.or(wallet).unwrap_or_default(), amount })
        .collect();

    let views_last_7d: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM "PageView" WHERE "appId" = $1 AND "createdAt" >= now() - interval '7 days'"#,
    )
    .bind(&app.id)
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let snapshot_rows: Vec<(NaiveDateTime, f64, f64, i32, f64)> = sqlx::query_as(
        r#"SELECT date, "voteWeight", "stakeTotal", "viewCount", "rankScore" FROM "AppStatsSnapshot" WHERE "appId" = $1 ORDER BY date ASC"#,
    )
    .bind(&app.id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;
    let snapshots = snapshot_rows
        .into_iter()
        .map(|(date, vote_weight, stake_total, view_count, rank_score)| SnapshotPointDto {
            date: to_rfc3339(date),
            vote_weight,
            stake_total,
            view_count,
            rank_score,
        })
        .collect();

    Ok(Json(AppDetailDto { app, recent_votes, top_stakers, views_last_7d, snapshots }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelatedQuery {
    #[serde(default)]
    slugs: Option<String>,
    #[serde(default)]
    tag_slugs: Option<String>,
}

const MAX_RELATED_RESULTS: usize = 24;

async fn related(
    State(state): State<Arc<ApiState>>,
    Query(q): Query<RelatedQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let slugs: Vec<String> = q.slugs.map(|s| s.split(',').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default();
    let tag_slugs: Vec<String> = q.tag_slugs.map(|s| s.split(',').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default();

    if slugs.is_empty() && tag_slugs.is_empty() {
        return Ok(Json(serde_json::json!({ "apps": [] })));
    }

    let mut apps = fetch_all_approved(&state.pool).await?;
    apps.retain(|a| {
        if !slugs.is_empty() {
            slugs.contains(&a.slug)
        } else {
            a.tags.iter().any(|t| tag_slugs.contains(&t.slug))
        }
    });
    apps.sort_by(|a, b| b.rank_score.partial_cmp(&a.rank_score).unwrap_or(std::cmp::Ordering::Equal));
    apps.truncate(MAX_RELATED_RESULTS);

    Ok(Json(serde_json::json!({ "apps": apps })))
}

// ---------------------------------------------------------------------
// POST /apps/search — search.ts + fuzzy.ts + ranking.ts, ported verbatim
// ---------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchInput {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub fuzzy: String,
    pub app_stake_min: Option<f64>,
    pub app_stake_max: Option<f64>,
    pub tags_stake_min: Option<f64>,
    pub tags_stake_max: Option<f64>,
    pub tags_count_min: Option<i64>,
    pub tags_count_max: Option<i64>,
    pub pageviews_min: Option<f64>,
    pub pageviews_max: Option<f64>,
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_sort() -> String { "rank".to_string() }
fn default_page() -> i64 { 1 }
fn default_page_size() -> i64 { 20 }

/// Every returned app's `trend`/AppCard subtext is always this window
/// ("weekly growth"), independent of `sort` — see this module's `search_apps`.
const TREND_DISPLAY_DAYS: i64 = 7;
/// The comparison window behind the "trending_month" sort option — see
/// `SORT_OPTIONS` in constants.ts for "Trending (week)"/"Trending (month)".
const TRENDING_MONTH_DAYS: i64 = 30;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub apps: Vec<AppDto>,
    pub total: usize,
    pub page: i64,
    pub page_size: i64,
    pub facets: FacetsDto,
}

#[derive(Serialize)]
pub struct FacetsDto {
    pub tags: Vec<TagFacetDto>,
}

#[derive(Serialize)]
pub struct TagFacetDto {
    pub slug: String,
    pub name: String,
    pub count: i64,
}

fn tokenize(q: &str) -> Vec<String> {
    q.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(String::from)
        .collect()
}

/// Same weighting as `search.ts`'s `textRelevance`.
fn text_relevance(app: &AppDto, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let name = app.name.to_lowercase();
    let tagline = app.tagline.to_lowercase();
    let description = app.description.to_lowercase();
    let tag_text = app.tags.iter().map(|t| t.name.to_lowercase()).collect::<Vec<_>>().join(" ");

    let mut score = 0.0;
    for term in terms {
        if name == *term {
            score += 5.0;
        } else if name.starts_with(term.as_str()) {
            score += 3.0;
        } else if name.contains(term.as_str()) {
            score += 2.0;
        }
        if tagline.contains(term.as_str()) {
            score += 1.2;
        }
        if tag_text.contains(term.as_str()) {
            score += 1.0;
        }
        if description.contains(term.as_str()) {
            score += 0.5;
        }
    }
    let max_per_term: f64 = 5.0 + 1.2 + 1.0 + 0.5;
    (score / (terms.len() as f64 * max_per_term)).min(1.0)
}

/// Same subsequence-matching heuristic as `fuzzy.ts`'s `fuzzyScore`.
fn fuzzy_score(text: &str, query: &str) -> f64 {
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let q: Vec<char> = query.to_lowercase().trim().chars().collect();
    if q.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    let mut ti = 0usize;
    let mut consecutive = 0i64;

    for &ch in &q {
        let Some(found_at) = t[ti..].iter().position(|&c| c == ch).map(|p| p + ti) else {
            return -1.0;
        };
        let is_boundary = found_at == 0 || !t[found_at - 1].is_ascii_alphanumeric();
        let is_consecutive = found_at == ti;
        consecutive = if is_consecutive { consecutive + 1 } else { 0 };
        score += 1.0 + (consecutive as f64) * 2.0 + if is_boundary { 2.0 } else { 0.0 };
        score -= (found_at.saturating_sub(ti)).min(5) as f64 * 0.2;
        ti = found_at + 1;
    }
    score
}

const MIN_FUZZY_SCORE_PER_CHAR: f64 = 1.2;

fn fuzzy_match(text: &str, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    fuzzy_score(text, q) >= q.chars().count() as f64 * MIN_FUZZY_SCORE_PER_CHAR
}

fn tags_stake_value(app: &AppDto, selected: &[String]) -> f64 {
    if !selected.is_empty() {
        app.tags.iter().filter(|t| selected.contains(&t.slug)).map(|t| t.stake_total).sum()
    } else {
        app.tags.iter().map(|t| t.stake_total).fold(0.0, f64::max)
    }
}

/// Shared with `handlers::platform`'s app-graph/tag-pack range filters
/// (the maps' "advanced search") — same range-check, applied over a
/// different app subset than `search_apps` below.
pub(crate) fn in_range(value: f64, min: Option<f64>, max: Option<f64>) -> bool {
    if let Some(min) = min {
        if value < min {
            return false;
        }
    }
    if let Some(max) = max {
        if value > max {
            return false;
        }
    }
    true
}

fn compute_facets(apps: &[AppDto]) -> FacetsDto {
    let mut tags: Vec<(String, String, i64)> = Vec::new();
    for app in apps {
        for t in &app.tags {
            if let Some(entry) = tags.iter_mut().find(|(slug, _, _)| slug == &t.slug) {
                entry.2 += 1;
            } else {
                tags.push((t.slug.clone(), t.name.clone(), 1));
            }
        }
    }
    tags.sort_by(|a, b| b.2.cmp(&a.2));
    tags.truncate(30);
    tags.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    FacetsDto {
        tags: tags.into_iter().map(|(slug, name, count)| TagFacetDto { slug, name, count }).collect(),
    }
}

/// `None` when `past` is exactly 0 — "grew from 0" has no percent to show,
/// and returning `Infinity`/a huge number there would be misleading rather
/// than informative (see TrendDto's doc comment).
fn pct_change(current: f64, past: f64) -> Option<f64> {
    if past == 0.0 {
        None
    } else {
        Some((current - past) / past * 100.0)
    }
}

struct TrendBaseline {
    vote_weight: f64,
    stake_total: f64,
    view_count: i32,
    rank_score: f64,
}

/// One query for every app in `app_ids` rather than N+1 (same reasoning as
/// `revenue.rs`'s `traffic` handler): the most recent `AppStatsSnapshot` row
/// at or before `now() - interval_days`, per app — `DISTINCT ON` picks the
/// single latest-but-still-old-enough row per `appId`. Apps with no
/// snapshot that old at all (created too recently for the cron to have
/// run, or a gap in cron history — see the daily-snapshot job) simply have
/// no entry in the returned map; callers treat a missing entry as "no trend
/// data" rather than assuming one exists.
async fn fetch_trend_baseline(
    pool: &PgPool,
    app_ids: &[String],
    interval_days: i64,
) -> Result<HashMap<String, TrendBaseline>, ApiError> {
    if app_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::days(interval_days.max(0));
    let rows: Vec<(String, f64, f64, i32, f64)> = sqlx::query_as(
        r#"
        SELECT DISTINCT ON ("appId") "appId", "voteWeight", "stakeTotal", "viewCount", "rankScore"
        FROM "AppStatsSnapshot"
        WHERE "appId" = ANY($1) AND date <= $2
        ORDER BY "appId", date DESC
        "#,
    )
    .bind(app_ids)
    .bind(cutoff)
    .fetch_all(pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(rows
        .into_iter()
        .map(|(app_id, vote_weight, stake_total, view_count, rank_score)| {
            (app_id, TrendBaseline { vote_weight, stake_total, view_count, rank_score })
        })
        .collect())
}

fn trend_for(app: &AppDto, baseline: &TrendBaseline, interval_days: i64) -> TrendDto {
    TrendDto {
        interval_days,
        vote_weight_pct: pct_change(app.vote_weight, baseline.vote_weight),
        stake_total_pct: pct_change(app.stake_total, baseline.stake_total),
        view_count_pct: pct_change(app.view_count as f64, baseline.view_count as f64),
        rank_score_pct: pct_change(app.rank_score, baseline.rank_score),
    }
}

/// The "trending" sort's comparator, pulled out as a pure function (id +
/// current rank_score in, `Ordering` out) so it's unit-testable without a
/// database. Top gainers first, by the ABSOLUTE change in rank_score (the
/// platform's own existing blended "how good is this app right now"
/// measure — reused here rather than inventing a second, competing
/// definition of "good"), not a percent change: a percent would let a tiny
/// app with a near-zero baseline (e.g. 0.001 -> 0.1, "+9900%") dominate
/// over a large app's genuinely significant 500 -> 550 ("+10%") gain, which
/// isn't what "trending" should mean. Apps with no baseline old enough to
/// compare against (too new, or a gap in the daily-snapshot cron history)
/// have nothing to claim as a gain, so they sort after every app that
/// does — among themselves, falling back to plain rank_score so they still
/// land somewhere sensible instead of an arbitrary order.
fn compare_trending(
    a_id: &str,
    a_rank_score: f64,
    b_id: &str,
    b_rank_score: f64,
    baseline: &HashMap<String, TrendBaseline>,
) -> std::cmp::Ordering {
    let da = baseline.get(a_id).map(|base| a_rank_score - base.rank_score);
    let db = baseline.get(b_id).map(|base| b_rank_score - base.rank_score);
    match (da, db) {
        (Some(da), Some(db)) => db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b_rank_score.partial_cmp(&a_rank_score).unwrap_or(std::cmp::Ordering::Equal),
    }
}

/// Same shape as `search.ts`'s `searchApps` — see this module's doc comment.
pub async fn search_apps(pool: &PgPool, input: &SearchInput) -> Result<SearchResult, ApiError> {
    let terms = tokenize(&input.q);
    let mut apps = fetch_all_approved(pool).await?;

    // Coarse filters `search.ts` used to push into the DB `WHERE` — applied
    // in-process here since we already fetched the full approved set above.
    apps.retain(|a| in_range(a.stake_total, input.app_stake_min, input.app_stake_max));
    apps.retain(|a| in_range(a.view_count as f64, input.pageviews_min, input.pageviews_max));
    if !input.tags.is_empty() {
        apps.retain(|a| input.tags.iter().all(|slug| a.tags.iter().any(|t| &t.slug == slug)));
    }

    if !terms.is_empty() {
        apps.retain(|a| text_relevance(a, &terms) > 0.0);
    }

    if input.tags_count_min.is_some() || input.tags_count_max.is_some() {
        apps.retain(|a| {
            in_range(
                a.tags.len() as f64,
                input.tags_count_min.map(|v| v as f64),
                input.tags_count_max.map(|v| v as f64),
            )
        });
    }

    if input.tags_stake_min.is_some() || input.tags_stake_max.is_some() {
        apps.retain(|a| in_range(tags_stake_value(a, &input.tags), input.tags_stake_min, input.tags_stake_max));
    }

    let fuzzy = input.fuzzy.trim();
    if !fuzzy.is_empty() {
        apps.retain(|a| fuzzy_match(&format!("{} {} {}", a.name, a.tagline, a.description), fuzzy));
    }

    // Trend data (AppCard's "+12%/7d" subtext) is always the weekly window,
    // attached to every returned app regardless of sort. One bulk query for
    // the whole filtered set, not one per app.
    let app_ids: Vec<String> = apps.iter().map(|a| a.id.clone()).collect();
    let weekly_baseline = fetch_trend_baseline(pool, &app_ids, TREND_DISPLAY_DAYS).await?;
    for app in &mut apps {
        if let Some(b) = weekly_baseline.get(&app.id) {
            app.trend = Some(trend_for(app, b, TREND_DISPLAY_DAYS));
        }
    }

    // "trending_month" compares against a 30-day-old baseline instead of the
    // weekly one above — fetched separately since it's only used to order
    // results, never shown (AppCard's subtext stays weekly regardless).
    let monthly_baseline = if input.sort == "trending_month" {
        Some(fetch_trend_baseline(pool, &app_ids, TRENDING_MONTH_DAYS).await?)
    } else {
        None
    };

    let max_rank = apps.iter().map(|a| a.rank_score).fold(0.0, f64::max);
    match input.sort.as_str() {
        "votes" => apps.sort_by(|a, b| b.vote_weight.partial_cmp(&a.vote_weight).unwrap()),
        "stake" => apps.sort_by(|a, b| b.stake_total.partial_cmp(&a.stake_total).unwrap()),
        "traffic" => apps.sort_by(|a, b| b.view_count.cmp(&a.view_count)),
        "new" => apps.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        // Top gainers first — see compare_trending's doc comment.
        "trending_week" => {
            apps.sort_by(|a, b| compare_trending(&a.id, a.rank_score, &b.id, b.rank_score, &weekly_baseline))
        }
        "trending_month" => {
            let baseline = monthly_baseline.as_ref().unwrap();
            apps.sort_by(|a, b| compare_trending(&a.id, a.rank_score, &b.id, b.rank_score, baseline))
        }
        _ => apps.sort_by(|a, b| {
            let sa = crate::handlers::engine::combine_search_score(text_relevance(a, &terms), a.rank_score, max_rank);
            let sb = crate::handlers::engine::combine_search_score(text_relevance(b, &terms), b.rank_score, max_rank);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        }),
    }

    let total = apps.len();
    let facets = compute_facets(&apps);

    let start = ((input.page - 1) * input.page_size).max(0) as usize;
    let page_apps: Vec<AppDto> = apps.into_iter().skip(start).take(input.page_size.max(0) as usize).collect();

    Ok(SearchResult { apps: page_apps, total, page: input.page, page_size: input.page_size, facets })
}

async fn search(
    State(state): State<Arc<ApiState>>,
    Json(input): Json<SearchInput>,
) -> Result<Json<SearchResult>, ApiError> {
    Ok(Json(search_apps(&state.pool, &input).await?))
}

// ---------------------------------------------------------------------
// OpenGraph backfill support — GET /apps/missing-metadata, PATCH /apps/:id/metadata
// ---------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MissingMetadataAppDto {
    id: String,
    slug: String,
    url: String,
    icon_url: Option<String>,
    tagline: String,
    description: String,
}

/// Apps missing icon/tagline/description — same filter `backfillOpengraph.ts`
/// used to build its Prisma `where`.
async fn missing_metadata(State(state): State<Arc<ApiState>>) -> Result<Json<Vec<MissingMetadataAppDto>>, ApiError> {
    let rows: Vec<(String, String, String, Option<String>, String, String)> = sqlx::query_as(
        r#"
        SELECT id, slug, url, "iconUrl", tagline, description
        FROM "App"
        WHERE "iconUrl" IS NULL OR tagline = '' OR description = ''
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|(id, slug, url, icon_url, tagline, description)| MissingMetadataAppDto { id, slug, url, icon_url, tagline, description })
            .collect(),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateMetadataReq {
    icon_url: Option<String>,
    tagline: Option<String>,
    description: Option<String>,
}

async fn update_metadata(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMetadataReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    crate::processors::product::apply_metadata_update(
        &state.pool,
        &id,
        req.icon_url.as_deref(),
        req.tagline.as_deref(),
        req.description.as_deref(),
    )
    .await
    .map_err(crate::api::internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new()
        .route("/apps/by-id/:id", get(get_by_id))
        .route("/apps/by-slug/:slug", get(get_by_slug))
        .route("/apps/related", get(related))
        .route("/apps/search", post(search))
        .route("/apps/missing-metadata", get(missing_metadata))
        .route("/apps/:id/metadata", patch(update_metadata))
}

/// Ported from `app/src/lib/fuzzy.test.ts` — same assertions, exercising the
/// Rust port instead of the deleted TS original.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_match_matches_an_exact_substring() {
        assert!(fuzzy_match("The Jupiter aggregator", "jupiter"));
    }

    #[test]
    fn fuzzy_match_matches_a_typod_partial_query_as_a_tight_subsequence() {
        assert!(fuzzy_match("The Jupiter aggregator", "jupitr"));
    }

    #[test]
    fn fuzzy_match_rejects_characters_out_of_order() {
        assert!(!fuzzy_match("Jupiter", "retipuj"));
    }

    #[test]
    fn fuzzy_match_rejects_a_scattered_match_across_unrelated_words() {
        assert!(!fuzzy_match("A general purpose onchain routing engine for token swaps", "jupiter"));
    }

    #[test]
    fn fuzzy_match_treats_an_empty_query_as_matching_everything() {
        assert!(fuzzy_match("anything at all", ""));
    }

    #[test]
    fn fuzzy_match_is_case_insensitive() {
        assert!(fuzzy_match("JUPITER AG", "jupiter"));
    }

    #[test]
    fn fuzzy_score_scores_tighter_matches_higher_than_looser_ones() {
        let tight = fuzzy_score("jupiter", "jupiter");
        let loose = fuzzy_score("j u p i t e r spread far apart", "jupiter");
        assert!(tight > loose);
    }

    #[test]
    fn fuzzy_score_returns_negative_one_when_not_a_subsequence() {
        assert_eq!(fuzzy_score("hello", "xyz"), -1.0);
    }

    #[test]
    fn pct_change_computes_a_standard_percent_increase() {
        assert_eq!(pct_change(110.0, 100.0), Some(10.0));
    }

    #[test]
    fn pct_change_computes_a_decrease_as_negative() {
        assert_eq!(pct_change(80.0, 100.0), Some(-20.0));
    }

    #[test]
    fn pct_change_is_none_for_a_zero_baseline() {
        assert_eq!(pct_change(5.0, 0.0), None);
    }

    fn baseline_map(entries: &[(&str, f64)]) -> HashMap<String, TrendBaseline> {
        entries
            .iter()
            .map(|(id, rank_score)| {
                (
                    id.to_string(),
                    TrendBaseline { vote_weight: 0.0, stake_total: 0.0, view_count: 0, rank_score: *rank_score },
                )
            })
            .collect()
    }

    #[test]
    fn compare_trending_ranks_the_bigger_absolute_gainer_first() {
        // a: 10 -> 20 (+10 points) outgains b: 1000 -> 1005 (+5 points),
        // regardless of b's much bigger starting point.
        let baseline = baseline_map(&[("a", 10.0), ("b", 1000.0)]);
        assert_eq!(compare_trending("a", 20.0, "b", 1005.0, &baseline), std::cmp::Ordering::Less);
    }

    #[test]
    fn compare_trending_does_not_let_a_tiny_baseline_inflate_a_percent_win() {
        // a: 0.001 -> 0.1 is a +9900% swing but only +0.099 points. b:
        // 500 -> 550 is "only" +10% but +50 points — a real, significant
        // gain. Absolute comparison correctly ranks b first; this is
        // exactly the distortion using rank_score_pct for sorting would
        // cause instead.
        let baseline = baseline_map(&[("a", 0.001), ("b", 500.0)]);
        assert_eq!(compare_trending("a", 0.1, "b", 550.0, &baseline), std::cmp::Ordering::Greater);
    }

    #[test]
    fn compare_trending_sorts_apps_with_no_baseline_after_apps_that_have_one() {
        let baseline = baseline_map(&[("has-history", 10.0)]);
        assert_eq!(
            compare_trending("has-history", 11.0, "brand-new", 999.0, &baseline),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_trending("brand-new", 999.0, "has-history", 11.0, &baseline),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn compare_trending_falls_back_to_rank_score_when_neither_has_a_baseline() {
        let baseline = baseline_map(&[]);
        assert_eq!(compare_trending("a", 5.0, "b", 10.0, &baseline), std::cmp::Ordering::Greater);
    }
}
