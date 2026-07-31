//! XP & Levels — cosmetic gamification layered on existing on-chain actions
//! (submit app, suggest tag, vote, stake) plus a once-per-UTC-day bonus. See
//! docs/plans/2026-07-20-gamification-xp-levels-design.md. Never touches
//! vote weight, fees, or ranking — status only.

pub const XP_SUBMIT_APP: i32 = 100;
pub const XP_SUGGEST_TAG: i32 = 40;
pub const XP_VOTE: i32 = 20;
pub const XP_STAKE: i32 = 30;
pub const XP_DAILY_BONUS: i32 = 15;

/// Cumulative XP required to REACH `level` (level 1 = 0 XP). Triangular
/// growth — each additional level costs a constant amount more than the
/// last (100, 200, 300, ...), so early levels come fast and later ones
/// stretch out. See design doc Section 3.
pub fn cumulative_xp_for_level(level: i32) -> i32 {
    50 * (level - 1) * level
}

pub fn level_for_xp(xp: i32) -> i32 {
    let mut level = 1;
    while cumulative_xp_for_level(level + 1) <= xp {
        level += 1;
    }
    level
}

pub fn title_for_level(level: i32) -> &'static str {
    match level {
        1..=4 => "Newcomer",
        5..=9 => "Regular",
        10..=19 => "Contributor",
        20..=29 => "Curator",
        30..=49 => "Tastemaker",
        _ => "Signal",
    }
}

use sqlx::PgPool;

use crate::api::{ApiError, ApiState};
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Inserts the XpEvent and bumps `User.xp` atomically (same transaction),
/// so a crash between the two statements can never leave a committed
/// XpEvent (which would permanently block a retry via the unique index's
/// `ON CONFLICT DO NOTHING`) with a `User.xp` that never got incremented.
/// Mirrors the `pool.begin()` / `tx.commit()` pattern used for the
/// settle-epoch invariant in `handlers/revenue.rs`.
///
/// `created_at` is when the underlying action actually happened, not
/// necessarily "now" — live callers (`award`) pass the current time,
/// `backfill` passes the historical row's own `createdAt` so a batch of
/// old actions lands on their true calendar days instead of all competing
/// for "today"'s single slot under the ("userId", kind, "awardDate")
/// unique index (see 007_xp_daily_cap.sql) that caps XP for a given
/// (user, kind) at once per UTC day.
async fn record_event(
    pool: &PgPool,
    user_id: &str,
    kind: &str,
    target_id: Option<&str>,
    amount: i32,
    created_at: NaiveDateTime,
) -> Result<bool, sqlx::Error> {
    let award_date = created_at.date();
    let mut tx = pool.begin().await?;

    let result = sqlx::query(
        r#"
        INSERT INTO "XpEvent" (id, "userId", kind, "targetId", amount, "createdAt", "awardDate")
        VALUES (gen_random_uuid()::text, $1, $2, $3, $4, $5, $6)
        ON CONFLICT ("userId", kind, "awardDate") DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(kind)
    .bind(target_id)
    .bind(amount)
    .bind(created_at)
    .bind(award_date)
    .execute(&mut *tx)
    .await?;

    let inserted = result.rows_affected() > 0;
    if inserted {
        sqlx::query(r#"UPDATE "User" SET xp = xp + $2 WHERE id = $1"#)
            .bind(user_id)
            .bind(amount)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    } else {
        // Nothing to persist — roll back the no-op transaction.
        tx.rollback().await?;
    }
    Ok(inserted)
}

/// The streak transition `award`'s UPDATE performs, in pure form — same
/// three rules, no database: a qualifying action the day after the last one
/// extends the run, any other gap restarts it at 1, and the personal best
/// only ever ratchets upward. `None` means "nothing to write" (today is
/// already recorded), mirroring that UPDATE's
/// `lastXpDate IS DISTINCT FROM today` guard.
///
/// Kept in sync with that SQL by hand, and pinned from both ends: the unit
/// tests below fix the intended transitions, and
/// `digest_integration::streak_*` runs the real statement against a real
/// Postgres and asserts it lands on the same numbers.
/// `#[cfg(test)]` because production never calls it — the SQL is the only
/// implementation that runs; this is its executable specification.
#[cfg(test)]
fn next_streak(
    last_xp_date: Option<chrono::NaiveDate>,
    today: chrono::NaiveDate,
    streak_days: i32,
    streak_best_days: i32,
) -> Option<(i32, i32)> {
    if last_xp_date == Some(today) {
        return None;
    }
    let days = if last_xp_date == Some(today - chrono::Duration::days(1)) {
        streak_days + 1
    } else {
        1
    };
    Some((days, streak_best_days.max(days)))
}

/// Stamps `lastXpDate` and advances the streak in ONE statement (see
/// docs/plans/2026-08-01-staker-digest-design.md): `award` already knows
/// "this user did a qualifying thing today" and already owns the
/// once-per-UTC-day boundary, so there is no second place that could
/// disagree about what day it is or double-count a day.
///
/// `lastXpDate IS DISTINCT FROM $2` makes the whole statement a no-op once
/// today is already recorded. That guard is load-bearing, not decoration:
/// the `daily_bonus` insert in `award` can legitimately LOSE the
/// `ON CONFLICT` race (its result is deliberately ignored — the unique index,
/// not the `lastXpDate` read, is the correctness boundary), and without the
/// guard the loser would re-evaluate the CASE against an already-advanced
/// `lastXpDate`, see "not yesterday", and reset a live streak to 1. With it,
/// whichever call gets there second changes nothing.
///
/// The CASE is repeated inside `GREATEST` rather than referencing
/// `"streakDays"`: within one UPDATE every right-hand column reference reads
/// the row's OLD value, so `GREATEST("streakBestDays", "streakDays")` would
/// ratchet the best against yesterday's count instead of today's.
///
/// `pub(crate)` only so the live-database test tier can run the real
/// statement twice and prove that second call is inert
/// (`digest_integration.rs`); nothing outside `award` should call it.
pub(crate) async fn mark_day_earned(
    pool: &PgPool,
    user_id: &str,
    today: chrono::NaiveDate,
) -> Result<(), sqlx::Error> {
    let yesterday = today - chrono::Duration::days(1);
    sqlx::query(
        r#"
        UPDATE "User" SET
            "lastXpDate" = $2,
            "streakDays" = CASE WHEN "lastXpDate" = $3 THEN "streakDays" + 1 ELSE 1 END,
            "streakBestDays" = GREATEST(
                "streakBestDays",
                CASE WHEN "lastXpDate" = $3 THEN "streakDays" + 1 ELSE 1 END
            )
        WHERE id = $1 AND "lastXpDate" IS DISTINCT FROM $2
        "#,
    )
    .bind(user_id)
    .bind(today)
    .bind(yesterday)
    .execute(pool)
    .await?;
    Ok(())
}

/// Awards XP for a fresh (wallet, kind) action today, plus the once-per-
/// UTC-day bonus if this wallet hasn't earned XP yet today. Best-effort by
/// design — callers log and swallow errors rather than failing the
/// underlying vote/stake/submit action over a gamification hiccup
/// (cosmetic only, must never block or roll back a real on-chain-backed
/// write).
pub async fn award(
    pool: &PgPool,
    user_id: &str,
    kind: &str,
    target_id: Option<&str>,
    amount: i32,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().naive_utc();
    if !record_event(pool, user_id, kind, target_id, amount, now).await? {
        return Ok(());
    }

    let today = now.date();
    let last_xp_date: Option<chrono::NaiveDate> =
        sqlx::query_scalar(r#"SELECT "lastXpDate" FROM "User" WHERE id = $1"#)
            .bind(user_id)
            .fetch_one(pool)
            .await?;

    if last_xp_date != Some(today) {
        // The ("userId", kind, "awardDate") unique index is the atomicity
        // boundary — two concurrent award() calls racing past the
        // lastXpDate read above both attempt this INSERT, but only one can
        // win under `ON CONFLICT DO NOTHING`. target_id can be None here
        // (unlike the old per-target index, "awardDate" is NOT NULL so
        // there's no null-never-equals-null gap to work around). The
        // lastXpDate read/write above remains a pure optimization to skip
        // the common case, not the correctness boundary.
        record_event(pool, user_id, "daily_bonus", None, XP_DAILY_BONUS, now).await?;
        mark_day_earned(pool, user_id, today).await?;
    }

    Ok(())
}

/// One-off historical backfill so existing users don't start at 0 XP when
/// this ships. Safe to call on every startup — `record_event`'s
/// `ON CONFLICT DO NOTHING` makes every call after the first a no-op. Each
/// row is dated to its own original `createdAt` (not "now") so a user's old
/// actions land on their true calendar days under the once-per-day cap,
/// rather than all competing for today's single slot per kind — ordered
/// oldest-first so, on the rare day a user did the same kind of action
/// twice historically, the earlier one is the one that wins the slot.
/// Deliberately does NOT grant daily bonuses for backfilled rows (there's no
/// meaningful "day" for a historical action being processed today).
pub async fn backfill(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let mut granted = 0;

    let votes: Vec<(String, String, NaiveDateTime)> = sqlx::query_as(
        r#"SELECT "userId", "appId", "createdAt" FROM "Vote" ORDER BY "createdAt" ASC"#,
    )
    .fetch_all(pool)
    .await?;
    for (user_id, app_id, created_at) in votes {
        if record_event(pool, &user_id, "vote", Some(&app_id), XP_VOTE, created_at).await? {
            granted += 1;
        }
    }

    let stakes: Vec<(String, String, NaiveDateTime)> = sqlx::query_as(
        r#"SELECT "userId", "appTagId", "createdAt" FROM "Stake" ORDER BY "createdAt" ASC"#,
    )
    .fetch_all(pool)
    .await?;
    for (user_id, app_tag_id, created_at) in stakes {
        if record_event(pool, &user_id, "stake", Some(&app_tag_id), XP_STAKE, created_at).await? {
            granted += 1;
        }
    }

    let apps: Vec<(String, String, NaiveDateTime)> = sqlx::query_as(
        r#"SELECT "submittedBy", id, "createdAt" FROM "App" WHERE "submittedBy" IS NOT NULL ORDER BY "createdAt" ASC"#,
    )
    .fetch_all(pool)
    .await?;
    for (user_id, app_id, created_at) in apps {
        if record_event(pool, &user_id, "submit_app", Some(&app_id), XP_SUBMIT_APP, created_at).await? {
            granted += 1;
        }
    }

    let tags: Vec<(String, String, NaiveDateTime)> = sqlx::query_as(
        r#"SELECT "suggestedBy", id, "createdAt" FROM "AppTag" WHERE "suggestedBy" IS NOT NULL ORDER BY "createdAt" ASC"#,
    )
    .fetch_all(pool)
    .await?;
    for (user_id, app_tag_id, created_at) in tags {
        if record_event(pool, &user_id, "suggest_tag", Some(&app_tag_id), XP_SUGGEST_TAG, created_at).await? {
            granted += 1;
        }
    }

    if granted > 0 {
        log::info!("xp backfill: granted {granted} historical XpEvents");
    }
    Ok(granted)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct XpDto {
    user_id: String,
    xp: i32,
    level: i32,
    title: &'static str,
    xp_into_level: i32,
    xp_for_next_level: i32,
    progress: f64,
    apps_submitted: i64,
    tags_suggested: i64,
    votes_cast: i64,
    stakes_made: i64,
    /// Task kinds (see XP_* consts) already earned today (UTC) —
    /// `daily_bonus` excluded since it isn't a user-initiated action a
    /// "still to do today" panel could link to. Drives the profile page's
    /// "earn more XP today" panel: whatever's NOT in this list is still
    /// available.
    xp_earned_today: Vec<String>,
}

async fn get_xp(
    State(state): State<Arc<ApiState>>,
    Path(user_id): Path<String>,
) -> Result<Json<XpDto>, ApiError> {
    let xp: i32 = sqlx::query_scalar(r#"SELECT xp FROM "User" WHERE id = $1"#)
        .bind(&user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(crate::api::internal)?
        .ok_or_else(|| crate::api::not_found("User not found"))?;

    let level = level_for_xp(xp);
    let title = title_for_level(level);
    let level_floor = cumulative_xp_for_level(level);
    let level_ceiling = cumulative_xp_for_level(level + 1);

    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
          COUNT(*) FILTER (WHERE kind = 'submit_app'),
          COUNT(*) FILTER (WHERE kind = 'suggest_tag'),
          COUNT(*) FILTER (WHERE kind = 'vote'),
          COUNT(*) FILTER (WHERE kind = 'stake')
        FROM "XpEvent" WHERE "userId" = $1
        "#,
    )
    .bind(&user_id)
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let today = chrono::Utc::now().naive_utc().date();
    let xp_earned_today: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT kind FROM "XpEvent"
        WHERE "userId" = $1 AND "awardDate" = $2 AND kind != 'daily_bonus'
        "#,
    )
    .bind(&user_id)
    .bind(today)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(Json(XpDto {
        user_id,
        xp,
        level,
        title,
        xp_into_level: xp - level_floor,
        xp_for_next_level: level_ceiling - level_floor,
        progress: (xp - level_floor) as f64 / (level_ceiling - level_floor) as f64,
        apps_submitted: counts.0,
        tags_suggested: counts.1,
        votes_cast: counts.2,
        stakes_made: counts.3,
        xp_earned_today,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct XpActivityEntry {
    id: String,
    kind: String,
    app_name: Option<String>,
    app_slug: Option<String>,
    tag_name: Option<String>,
    amount: i32,
    created_at: String,
}

type ActivityRow = (
    String,
    String,
    i32,
    NaiveDateTime,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn get_activity(
    State(state): State<Arc<ApiState>>,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows: Vec<ActivityRow> = sqlx::query_as(
        r#"
        SELECT
          e.id, e.kind, e.amount, e."createdAt",
          COALESCE(a.name, a2.name) AS app_name,
          COALESCE(a.slug, a2.slug) AS app_slug,
          t.name AS tag_name
        FROM "XpEvent" e
        LEFT JOIN "App" a ON e.kind IN ('vote', 'submit_app') AND a.id = e."targetId"
        LEFT JOIN "AppTag" at ON e.kind IN ('stake', 'suggest_tag') AND at.id = e."targetId"
        LEFT JOIN "App" a2 ON at."appId" = a2.id
        LEFT JOIN "Tag" t ON at."tagId" = t.id
        WHERE e."userId" = $1
        ORDER BY e."createdAt" DESC
        LIMIT 50
        "#,
    )
    .bind(&user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let events: Vec<XpActivityEntry> = rows
        .into_iter()
        .map(
            |(id, kind, amount, created_at, app_name, app_slug, tag_name)| XpActivityEntry {
                id,
                kind,
                app_name,
                app_slug,
                tag_name,
                amount,
                created_at: crate::handlers::engine::to_rfc3339(created_at),
            },
        )
        .collect();

    Ok(Json(serde_json::json!({ "events": events })))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LeaderboardEntry {
    user_id: String,
    wallet: String,
    handle: Option<String>,
    xp: i32,
    level: i32,
    title: &'static str,
}

#[derive(Deserialize)]
struct LeaderboardQuery {
    limit: Option<i64>,
}

/// Top wallets by lifetime XP — cosmetic status only, same as everything
/// else in this module (never derived from or feeding into vote weight,
/// stake, or rank score). `limit` defaults to 10 and is clamped to 50 to
/// keep this a cheap, bounded read.
async fn get_leaderboard(
    State(state): State<Arc<ApiState>>,
    Query(q): Query<LeaderboardQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = q.limit.unwrap_or(10).clamp(1, 50);

    let rows: Vec<(String, String, Option<String>, i32)> = sqlx::query_as(
        r#"
        SELECT id, wallet, handle, xp
        FROM "User"
        WHERE xp > 0
        ORDER BY xp DESC, id ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let entries: Vec<LeaderboardEntry> = rows
        .into_iter()
        .map(|(user_id, wallet, handle, xp)| {
            let level = level_for_xp(xp);
            LeaderboardEntry {
                user_id,
                wallet,
                handle,
                xp,
                level,
                title: title_for_level(level),
            }
        })
        .collect();

    Ok(Json(serde_json::json!({ "entries": entries })))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new()
        .route("/xp/leaderboard", get(get_leaderboard))
        .route("/xp/:user_id", get(get_xp))
        .route("/xp/:user_id/activity", get(get_activity))
}

#[cfg(test)]
mod curve_tests {
    use super::*;

    #[test]
    fn level_1_starts_at_zero() {
        assert_eq!(cumulative_xp_for_level(1), 0);
        assert_eq!(level_for_xp(0), 1);
    }

    #[test]
    fn matches_design_doc_table() {
        assert_eq!(cumulative_xp_for_level(2), 100);
        assert_eq!(cumulative_xp_for_level(3), 300);
        assert_eq!(cumulative_xp_for_level(4), 600);
        assert_eq!(cumulative_xp_for_level(5), 1000);
    }

    #[test]
    fn level_for_xp_is_the_floor_of_the_curve() {
        assert_eq!(level_for_xp(99), 1);
        assert_eq!(level_for_xp(100), 2);
        assert_eq!(level_for_xp(299), 2);
        assert_eq!(level_for_xp(300), 3);
    }

    fn day(n: i64) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 8, 1).expect("valid date") + chrono::Duration::days(n)
    }

    #[test]
    fn the_first_ever_qualifying_day_starts_a_streak_of_one() {
        assert_eq!(next_streak(None, day(0), 0, 0), Some((1, 1)));
    }

    #[test]
    fn a_consecutive_day_extends_the_streak() {
        assert_eq!(next_streak(Some(day(0)), day(1), 4, 9), Some((5, 9)));
    }

    #[test]
    fn a_gap_of_one_day_restarts_the_streak_at_one() {
        assert_eq!(next_streak(Some(day(0)), day(2), 4, 9), Some((1, 9)));
    }

    #[test]
    fn a_gap_of_many_days_restarts_the_streak_at_one() {
        assert_eq!(next_streak(Some(day(0)), day(90), 40, 40), Some((1, 40)));
    }

    #[test]
    fn the_personal_best_ratchets_when_the_streak_passes_it() {
        assert_eq!(next_streak(Some(day(0)), day(1), 9, 9), Some((10, 10)));
    }

    #[test]
    fn the_personal_best_never_falls_when_a_streak_breaks() {
        assert_eq!(next_streak(Some(day(0)), day(5), 3, 12), Some((1, 12)));
    }

    #[test]
    fn a_second_award_on_the_same_day_changes_nothing() {
        assert_eq!(next_streak(Some(day(1)), day(1), 5, 9), None);
    }

    #[test]
    fn titles_match_level_ranges() {
        assert_eq!(title_for_level(1), "Newcomer");
        assert_eq!(title_for_level(4), "Newcomer");
        assert_eq!(title_for_level(5), "Regular");
        assert_eq!(title_for_level(10), "Contributor");
        assert_eq!(title_for_level(20), "Curator");
        assert_eq!(title_for_level(30), "Tastemaker");
        assert_eq!(title_for_level(50), "Signal");
        assert_eq!(title_for_level(1000), "Signal");
    }
}
