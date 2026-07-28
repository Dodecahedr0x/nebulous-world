//! Session-outcome persistence — the IO-adjacent write adapter.
//!
//! This is the only module under `find/` that may touch `sqlx`; `scoring`,
//! `selection` and `blend` stay pure so the maths is testable with no database.
//! The split is load-bearing here too: every SQL statement below is deliberately
//! trivial, and all the arithmetic lives in [`aggregate_outcomes`], which is
//! pure and is where this module's tests are. There is no test that opens a
//! connection.
//!
//! Runtime `sqlx::query*` APIs only, never the `query!` macros — the macros need
//! a live database or a checked-in offline cache at compile time, which this
//! repo does not have. Every sibling in `handlers/` is written the same way.

use crate::api::ApiError;
use crate::find::blend::{normalize_outcome_mean, outcome_weight};
use crate::find::{Answer, FacetKind};
use sqlx::types::Json as SqlxJson;
use sqlx::PgPool;
use std::collections::HashMap;

/// The only statement that feeds the learned term, pinned as a constant so the
/// test below can assert its exact text. There is no database here to execute
/// it against (A72), so the strongest available guard on `WHERE
/// "turnstileVerified"` is that removing or inverting it changes this string.
const LEARNED_QUERY: &str =
    r#"SELECT "appId", "outcome" FROM "FindSession" WHERE "turnstileVerified""#;

/// `(support v_i, outcome mean R_i in [0, 1])` per app id, for
/// `facets::candidates_from_apps`. Apps with no logged outcome are absent, which
/// is what lets the caller distinguish "no history" from "bad history" — a
/// distinction the shrinkage term in `blend` depends on.
///
/// Unverified rows are written but never trained on (A80). The filter is a
/// `WHERE` on the existing scan rather than a second query, because A40 already
/// records that this reads every `"FindSession"` row with no window and no
/// `LIMIT`, and a fix for that must not have to unpick two scans first.
pub async fn load_learned(pool: &PgPool) -> Result<HashMap<String, (f64, f64)>, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as(LEARNED_QUERY)
        .fetch_all(pool)
        .await
        .map_err(crate::api::internal)?;

    Ok(aggregate_outcomes(&rows))
}

/// Pinned for the same reason as [`LEARNED_QUERY`]: `"turnstileVerified"` is
/// only useful if it is actually written, and no test here can execute the
/// insert to find out.
const RECORD_QUERY: &str = r#"
        INSERT INTO "FindSession" (id, "visitorId", "sessionId", "appId", "outcome", "answers", "questionsAsked", "turnstileVerified", "createdAt")
        VALUES (gen_random_uuid()::text, $1, $2, $3, $4, $5, $6, $7, now())
        ON CONFLICT ("sessionId", "appId", "outcome") DO NOTHING
        RETURNING id
        "#;

/// Records one completed funnel outcome. Returns `false` when the row already
/// existed — a replay of the same (`sessionId`, `appId`, `outcome`), which must
/// update nothing.
///
/// The replay check is the unique index doing the work, not an `if` here: a
/// second write path that forgot the check would reopen confirmation farming,
/// and the database is the one place that cannot be bypassed.
///
/// `turnstile_verified` is stored, never gated on (A80): a row whose token
/// failed is still written, and [`load_learned`] is what declines to train on
/// it. `DO NOTHING` is kept over an upsert of the flag, so a replay stays a
/// true no-op — a conflicting row that arrives verified cannot promote an
/// earlier unverified one, which keeps the loss visible instead of repairable
/// by re-POSTing.
#[allow(clippy::too_many_arguments)]
pub async fn record_outcome(
    pool: &PgPool,
    visitor_id: &str,
    session_id: &str,
    app_id: &str,
    outcome: &str,
    answers: &[Answer],
    questions_asked: i32,
    turnstile_verified: bool,
) -> Result<bool, ApiError> {
    let inserted: Option<String> = sqlx::query_scalar(RECORD_QUERY)
        .bind(visitor_id)
        .bind(session_id)
        .bind(app_id)
        .bind(outcome)
        .bind(SqlxJson(answers))
        .bind(questions_asked)
        .bind(turnstile_verified)
        .fetch_optional(pool)
        .await
        .map_err(crate::api::internal)?;

    Ok(inserted.is_some())
}

/// Upserts the yes/no/skip tallies for each answered facet.
///
/// One statement per answer, with the increment carried as three bound 0/1
/// values rather than three column-specific SQL strings — a column name cannot
/// be a bind parameter, and building the statement with `format!` would put
/// user-adjacent data into SQL text for no gain.
pub async fn bump_facet_stats(pool: &PgPool, answers: &[Answer]) -> Result<(), ApiError> {
    for answer in answers {
        let (yes, no, skip) = match answer.value {
            crate::find::AnswerValue::Yes => (1, 0, 0),
            crate::find::AnswerValue::No => (0, 1, 0),
            crate::find::AnswerValue::Skip => (0, 0, 1),
        };

        sqlx::query(
            r#"
            INSERT INTO "FindFacetStat" ("facetKind", "facetValue", "yesCount", "noCount", "skipCount", "updatedAt")
            VALUES ($1, $2, $3, $4, $5, now())
            ON CONFLICT ("facetKind", "facetValue") DO UPDATE SET
                "yesCount" = "FindFacetStat"."yesCount" + EXCLUDED."yesCount",
                "noCount" = "FindFacetStat"."noCount" + EXCLUDED."noCount",
                "skipCount" = "FindFacetStat"."skipCount" + EXCLUDED."skipCount",
                "updatedAt" = now()
            "#,
        )
        .bind(facet_kind_key(answer.facet.kind))
        .bind(&answer.facet.value)
        .bind(yes)
        .bind(no)
        .bind(skip)
        .execute(pool)
        .await
        .map_err(crate::api::internal)?;
    }

    Ok(())
}

/// The stored spelling of a facet kind, taken from `serde` rather than
/// hand-written, so the `"facetKind"` column can never drift from the same
/// enum's spelling inside `FindSession.answers` or on the wire.
fn facet_kind_key(kind: FacetKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .expect("FacetKind is a fieldless enum and serializes to a JSON string")
}

/// Deliberately *not* filtered to `"turnstileVerified"`, unlike
/// [`LEARNED_QUERY`]: this measures how many questions the funnel takes, which
/// is a fact about the funnel and not about how much a row is trusted.
/// Excluding unverified rows here would reintroduce the sample bias A80 exists
/// to remove, in the metric instead of in the training set — and it is the
/// metric that makes the size of that bias visible.
///
/// The `::double precision` cast is not cosmetic: `AVG` over an `INTEGER`
/// column is `NUMERIC` in Postgres, which sqlx will not hand back as `f64`
/// without the bigdecimal feature.
const AVG_QUESTIONS_QUERY: &str = r#"SELECT AVG("questionsAsked")::double precision FROM "FindSession" WHERE "outcome" = 'confirmed'"#;

/// Brief success criterion 7 — average questions-to-confirm, so the self-tuning
/// loop's effect is observable rather than assumed. `None` when no session has
/// confirmed yet: zero would read as "instant success", which is the opposite of
/// what no data means. See [`AVG_QUESTIONS_QUERY`] for what it does and does not
/// filter.
pub async fn avg_questions_to_confirm(pool: &PgPool) -> Result<Option<f64>, ApiError> {
    sqlx::query_scalar::<_, Option<f64>>(AVG_QUESTIONS_QUERY)
        .fetch_one(pool)
        .await
        .map_err(crate::api::internal)
}

/// Pure — the aggregation [`load_learned`] applies to raw `(appId, outcome)`
/// rows. Exposed so it can be unit-tested without a database, which is the only
/// reason the SQL above is a bare `SELECT` rather than a `GROUP BY`.
///
/// An outcome string `blend` has no weight for is skipped entirely and does not
/// count toward `support`, so an app whose rows are all unrecognised is absent
/// from the map. That is deliberate: `support` feeds the shrinkage ratio
/// `v/(v+m)`, and letting rows the mean never sees inflate `v` would move the
/// learned term's confidence without any evidence behind it.
pub fn aggregate_outcomes(rows: &[(String, String)]) -> HashMap<String, (f64, f64)> {
    let mut totals: HashMap<&str, (f64, f64)> = HashMap::new();
    for (app_id, outcome) in rows {
        let Some(weight) = outcome_weight(outcome) else {
            continue;
        };
        let entry = totals.entry(app_id).or_insert((0.0, 0.0));
        entry.0 += 1.0;
        entry.1 += weight;
    }

    totals
        .into_iter()
        .map(|(app_id, (support, weight_sum))| {
            (
                app_id.to_string(),
                (support, normalize_outcome_mean(weight_sum / support)),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, o)| (a.to_string(), o.to_string()))
            .collect()
    }

    /// The schema this module's SQL is written against. Compiled in so the two
    /// cannot drift silently — `cargo check` treats a query string as opaque
    /// (A72), so nothing else in this crate would notice.
    const MIGRATION: &str = include_str!("../../migrations/009_find_funnel.sql");

    /// A80's load-bearing guard: an outcome recorded with a failed Turnstile
    /// must never reach the learned weights. Pinned as exact text because there
    /// is no database to run it against — a `contains("turnstileVerified")`
    /// would still pass if the predicate were negated.
    #[test]
    fn learned_query_trains_only_on_verified_rows_a78() {
        assert_eq!(
            LEARNED_QUERY,
            r#"SELECT "appId", "outcome" FROM "FindSession" WHERE "turnstileVerified""#
        );
        // A40: one scan, not two. A `UNION`/second `SELECT` would double it.
        assert_eq!(LEARNED_QUERY.matches("SELECT").count(), 1);
    }

    /// The other half: the flag is worthless if the insert never writes it, and
    /// the column and its bind position have to move together.
    #[test]
    fn record_query_writes_the_verified_flag_a78() {
        assert!(RECORD_QUERY.contains(r#""questionsAsked", "turnstileVerified", "createdAt""#));
        assert!(RECORD_QUERY.contains("$1, $2, $3, $4, $5, $6, $7, now()"));
        // A19: a replay stays a true no-op, so it cannot promote an unverified
        // row to verified.
        assert!(
            RECORD_QUERY.contains(r#"ON CONFLICT ("sessionId", "appId", "outcome") DO NOTHING"#)
        );
    }

    /// The mirror image of the two above, and the one people will be tempted to
    /// "fix": criterion 7's metric must keep counting unverified rows, because
    /// it is the only place the size of the Turnstile loss shows up. Filtering
    /// it would hide exactly what A80 was written to make visible.
    #[test]
    fn the_criterion_7_metric_counts_unverified_rows_too_a78() {
        assert_eq!(
            AVG_QUESTIONS_QUERY,
            r#"SELECT AVG("questionsAsked")::double precision FROM "FindSession" WHERE "outcome" = 'confirmed'"#
        );
    }

    /// Every identifier the statements above name, checked against the
    /// migration itself. This is review-verification made executable; it is
    /// still not execution (A72).
    #[test]
    fn migration_declares_every_column_the_queries_name() {
        for column in [
            r#""appId" TEXT NOT NULL"#,
            r#""outcome" TEXT NOT NULL"#,
            r#""questionsAsked" INTEGER NOT NULL"#,
            r#""turnstileVerified" BOOLEAN NOT NULL DEFAULT true"#,
        ] {
            assert!(MIGRATION.contains(column), "009 is missing: {column}");
        }
    }

    #[test]
    fn empty_input_yields_empty_map() {
        assert!(aggregate_outcomes(&[]).is_empty());
    }

    #[test]
    fn single_confirm_is_full_support_and_top_of_range() {
        let learned = aggregate_outcomes(&rows(&[("a", "confirmed")]));
        assert_eq!(learned.get("a"), Some(&(1.0, 1.0)));
    }

    #[test]
    fn single_rejection_is_full_support_and_bottom_of_range() {
        let learned = aggregate_outcomes(&rows(&[("a", "rejected")]));
        assert_eq!(learned.get("a"), Some(&(1.0, 0.0)));
    }

    #[test]
    fn mixed_outcomes_land_strictly_between() {
        let learned = aggregate_outcomes(&rows(&[("a", "confirmed"), ("a", "rejected")]));
        let (support, mean) = *learned.get("a").expect("app a has outcomes");
        assert_eq!(support, 2.0);
        assert!(mean > 0.0 && mean < 1.0, "mixed outcomes gave {mean}");
    }

    /// An outcome string the blend has no weight for is not a weak signal, it is
    /// no signal — counting it toward `support` would let a caller inflate the
    /// shrinkage ratio `v/(v+m)` with rows the mean itself never sees.
    #[test]
    fn unrecognised_outcomes_are_skipped_entirely() {
        let learned = aggregate_outcomes(&rows(&[("a", "upvoted")]));
        assert!(!learned.contains_key("a"));

        let learned = aggregate_outcomes(&rows(&[("b", "confirmed"), ("b", "upvoted")]));
        let (support, mean) = *learned.get("b").expect("app b has one recognised outcome");
        assert_eq!(support, 1.0);
        assert_eq!(mean, 1.0);
    }

    #[test]
    fn app_ids_stay_independent() {
        let learned = aggregate_outcomes(&rows(&[("a", "confirmed"), ("b", "rejected")]));
        assert_eq!(learned.get("a"), Some(&(1.0, 1.0)));
        assert_eq!(learned.get("b"), Some(&(1.0, 0.0)));
    }

    /// `outcome_mean` feeds the blend's `R_i`, whose shrinkage formula assumes
    /// `[0, 1]`. A raw weight escaping that range (`WEIGHT_REJECTED` is negative)
    /// would let the learned term subtract without bound and defeat the `LAMBDA`
    /// cap, so the range is asserted over every shape above rather than only the
    /// two endpoints.
    #[test]
    fn outcome_mean_never_leaves_the_unit_interval() {
        let cases = [
            vec![("a", "confirmed")],
            vec![("a", "rejected")],
            vec![("a", "clicked")],
            vec![("a", "confirmed"), ("a", "rejected")],
            vec![("a", "rejected"), ("a", "rejected"), ("a", "clicked")],
            vec![("a", "confirmed"), ("a", "upvoted")],
        ];
        for case in cases {
            for (_, mean) in aggregate_outcomes(&rows(&case)).values() {
                assert!((0.0..=1.0).contains(mean), "{case:?} gave mean {mean}");
            }
        }
    }
}
