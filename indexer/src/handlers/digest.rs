//! The staker digest — "since you were last here". See
//! docs/plans/2026-08-01-staker-digest-design.md.
//!
//! One `GET /digest?userId=…` computes everything the navbar bell shows:
//! outstanding claimable rewards, rank moves on the apps the user stakes,
//! and streak status. Deliberately computed HERE rather than in the app, so
//! a delivery channel (email, push) could consume the same payload later
//! without recomputing any of it. `POST /digest/seen` advances the
//! watermark the rank-move half is filtered against.
//!
//! The streak counters this reads are written by `handlers::xp::award` —
//! this module never mutates them (see the design doc's "Streak" section:
//! `award` already owns the once-per-UTC-day atomicity boundary).

use crate::api::{ApiError, ApiState};
use axum::extract::{Json, Query, State};
use axum::routing::{get, post};
use axum::Router;
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Rank moves smaller than this many positions are noise, not news — a
/// one-place shuffle happens to a mid-catalog app almost every snapshot.
const RANK_MOVE_MIN_DELTA: i64 = 2;

/// At most this many rank-move rows, biggest absolute move first. The panel
/// is a digest, not a report.
const RANK_MOVE_LIMIT: usize = 3;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DigestQuery {
    user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeenReq {
    user_id: String,
}

/// The render order of the `items` array IS this enum's construction order:
/// rewards, then rank moves, then streak (design doc, "DTO contract").
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DigestItem {
    #[serde(rename_all = "camelCase")]
    Reward {
        app_id: String,
        app_slug: String,
        app_name: String,
        app_icon_url: Option<String>,
        /// `f64`, matching `handlers/rewards.rs`'s `amount` exactly:
        /// `RevenueClaim.amount`/`RevenueEpoch.grossRevenue` are
        /// `DOUBLE PRECISION` off-chain accounting columns (005_app_schema.sql),
        /// not the on-chain u64/u128 token amounts that go over the wire as
        /// decimal strings (see api.rs's `PositionDto`).
        amount: f64,
        epoch_count: i64,
    },
    #[serde(rename_all = "camelCase")]
    RankMove {
        app_id: String,
        app_slug: String,
        app_name: String,
        app_icon_url: Option<String>,
        from: i64,
        to: i64,
        /// Positive = moved UP the board (`from` - `to`, so 7 -> 4 is +3).
        /// Down-moves are emitted too, and are the whole reason this is
        /// signed rather than absolute.
        delta: i64,
    },
    #[serde(rename_all = "camelCase")]
    Streak {
        streak_days: i32,
        best_days: i32,
        bonus_claimed_today: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DigestDto {
    /// Null if the user has never opened the panel.
    seen_at: Option<String>,
    count: usize,
    items: Vec<DigestItem>,
}

/// One app's rank at the watermark vs. now, before the noise guards.
#[derive(Debug, Clone, PartialEq)]
struct RankMove {
    app_id: String,
    app_slug: String,
    app_name: String,
    app_icon_url: Option<String>,
    from: i64,
    to: i64,
}

impl RankMove {
    fn delta(&self) -> i64 {
        self.from - self.to
    }
}

/// The two noise guards from the design doc, kept pure so they can be tested
/// without a database: drop anything that moved fewer than
/// `RANK_MOVE_MIN_DELTA` positions in either direction, then keep the
/// `RANK_MOVE_LIMIT` biggest moves by ABSOLUTE delta — a three-place slide
/// competes with a three-place climb on equal terms, because a staker who
/// cannot see a position slipping cannot act on it.
fn select_rank_moves(mut moves: Vec<RankMove>) -> Vec<RankMove> {
    moves.retain(|m| m.delta().abs() >= RANK_MOVE_MIN_DELTA);
    // Ties broken by app_id so the panel is stable between two loads that
    // saw the same data.
    moves.sort_by(|a, b| {
        b.delta()
            .abs()
            .cmp(&a.delta().abs())
            .then_with(|| a.app_id.cmp(&b.app_id))
    });
    moves.truncate(RANK_MOVE_LIMIT);
    moves
}

/// The badge number. The streak row is always SHOWN when the user has any
/// streak state, but only COUNTED when today's bonus is still unearned —
/// i.e. when the streak is actually at risk. Otherwise the bell would wear a
/// permanent "1" and stop meaning anything.
fn badge_count(rewards: usize, rank_moves: usize, streak_bonus_claimed_today: Option<bool>) -> usize {
    let streak = match streak_bonus_claimed_today {
        Some(false) => 1,
        _ => 0,
    };
    rewards + rank_moves + streak
}

type RewardRow = (String, String, String, Option<String>, f64, i64);

/// Outstanding claimable rewards — money is a STATE, not an event, so this
/// half is deliberately NOT watermark-filtered: an amount you saw yesterday
/// and did not claim must still be here today.
///
/// Deviates from the design doc's illustrative SQL in one place, deliberately
/// (see this file's commit/report): the doc selects `RevenueEpoch` rows the
/// user has NO `RevenueClaim` for, but `handlers/revenue.rs::settle_epoch`
/// inserts exactly one `RevenueClaim` per participant AT SETTLE TIME, so that
/// predicate would exclude every epoch the user is actually owed money from
/// and the item would always be empty. It also has no per-user amount to
/// report (`RevenueEpoch.grossRevenue` is the whole app's revenue, not this
/// user's share). Keying off the unclaimed `RevenueClaim` row instead gives
/// both the right set AND the right amount, and keeps the doc's
/// "already-claimed epochs are excluded" semantics.
async fn outstanding_rewards(
    pool: &sqlx::PgPool,
    user_id: &str,
) -> Result<Vec<DigestItem>, ApiError> {
    let rows: Vec<RewardRow> = sqlx::query_as(
        r#"
        SELECT a.id, a.slug, a.name, a."iconUrl",
               SUM(c.amount)::double precision AS amount,
               COUNT(*) AS epoch_count
        FROM "RevenueClaim" c
        JOIN "RevenueEpoch" e ON e.id = c."epochId"
        JOIN "App" a ON a.id = e."appId"
        WHERE c."userId" = $1
          AND c.claimed = false
          AND e.distributed = true
          AND EXISTS (
              SELECT 1 FROM "Stake" s
              JOIN "AppTag" at ON at.id = s."appTagId"
              WHERE at."appId" = e."appId" AND s."userId" = $1 AND s.active = true
          )
        GROUP BY a.id, a.slug, a.name, a."iconUrl"
        ORDER BY amount DESC, a.id ASC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(rows
        .into_iter()
        .map(|(app_id, app_slug, app_name, app_icon_url, amount, epoch_count)| DigestItem::Reward {
            app_id,
            app_slug,
            app_name,
            app_icon_url,
            amount,
            epoch_count,
        })
        .collect())
}

type RankRow = (String, String, String, Option<String>, i64, i64);

/// Rank moves on the user's actively staked apps, watermark-filtered:
/// today's POSITION (not raw `rankScore` — position is the legible unit)
/// against the position in the newest `AppStatsSnapshot` at or before
/// `seen_at`.
///
/// With no watermark, or no snapshot at or before it, this emits nothing
/// rather than comparing against the beginning of time.
async fn rank_moves(
    pool: &sqlx::PgPool,
    user_id: &str,
    seen_at: Option<NaiveDateTime>,
) -> Result<Vec<DigestItem>, ApiError> {
    let Some(seen_at) = seen_at else {
        return Ok(Vec::new());
    };

    let rows: Vec<RankRow> = sqlx::query_as(
        r#"
        WITH baseline_date AS (
            SELECT MAX("date") AS d FROM "AppStatsSnapshot" WHERE "date" <= $2
        ), baseline AS (
            SELECT s."appId",
                   RANK() OVER (PARTITION BY s."date" ORDER BY s."rankScore" DESC) AS pos
            FROM "AppStatsSnapshot" s
            JOIN baseline_date b ON s."date" = b.d
        ), current AS (
            SELECT a.id, a.slug, a.name, a."iconUrl",
                   RANK() OVER (ORDER BY a."rankScore" DESC) AS pos
            FROM "App" a
        )
        SELECT c.id, c.slug, c.name, c."iconUrl", b.pos, c.pos
        FROM current c
        JOIN baseline b ON b."appId" = c.id
        WHERE EXISTS (
            SELECT 1 FROM "Stake" s
            JOIN "AppTag" at ON at.id = s."appTagId"
            WHERE at."appId" = c.id AND s."userId" = $1 AND s.active = true
        )
        "#,
    )
    .bind(user_id)
    .bind(seen_at)
    .fetch_all(pool)
    .await
    .map_err(crate::api::internal)?;

    let moves = rows
        .into_iter()
        .map(|(app_id, app_slug, app_name, app_icon_url, from, to)| RankMove {
            app_id,
            app_slug,
            app_name,
            app_icon_url,
            from,
            to,
        })
        .collect();

    Ok(select_rank_moves(moves)
        .into_iter()
        .map(|m| DigestItem::RankMove {
            from: m.from,
            to: m.to,
            delta: m.delta(),
            app_id: m.app_id,
            app_slug: m.app_slug,
            app_name: m.app_name,
            app_icon_url: m.app_icon_url,
        })
        .collect())
}

async fn get_digest(
    State(state): State<Arc<ApiState>>,
    Query(q): Query<DigestQuery>,
) -> Result<Json<DigestDto>, ApiError> {
    type UserRow = (Option<NaiveDateTime>, i32, i32, Option<NaiveDate>);
    let user: Option<UserRow> = sqlx::query_as(
        r#"SELECT "digestSeenAt", "streakDays", "streakBestDays", "lastXpDate" FROM "User" WHERE id = $1"#,
    )
    .bind(&q.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let (seen_at, streak_days, best_days, last_xp_date) =
        user.ok_or_else(|| crate::api::not_found("User not found"))?;

    let mut items = outstanding_rewards(&state.pool, &q.user_id).await?;
    let rewards = items.len();

    let moves = rank_moves(&state.pool, &q.user_id, seen_at).await?;
    let rank_moves_count = moves.len();
    items.extend(moves);

    // The daily bonus is AUTO-awarded on any qualifying action (see
    // xp::award) — `lastXpDate == today` is what "already earned today"
    // means, there is no button to press.
    let today = chrono::Utc::now().naive_utc().date();
    let bonus_claimed_today = last_xp_date == Some(today);
    let streak_shown = streak_days > 0 || best_days > 0;
    if streak_shown {
        items.push(DigestItem::Streak {
            streak_days,
            best_days,
            bonus_claimed_today,
        });
    }

    Ok(Json(DigestDto {
        seen_at: seen_at.map(crate::handlers::engine::to_rfc3339),
        count: badge_count(
            rewards,
            rank_moves_count,
            streak_shown.then_some(bonus_claimed_today),
        ),
        items,
    }))
}

/// Advances the watermark. The panel the caller is currently looking at keeps
/// its contents — this only affects the NEXT load — and rewards, being state
/// rather than event, survive the advance regardless.
async fn post_seen(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SeenReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let seen_at: Option<NaiveDateTime> = sqlx::query_scalar(
        r#"UPDATE "User" SET "digestSeenAt" = now() WHERE id = $1 RETURNING "digestSeenAt""#,
    )
    .bind(&req.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(crate::api::internal)?
    .flatten();

    let seen_at = seen_at.ok_or_else(|| crate::api::not_found("User not found"))?;
    Ok(Json(serde_json::json!({
        "seenAt": crate::handlers::engine::to_rfc3339(seen_at),
    })))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new()
        .route("/digest", get(get_digest))
        .route("/digest/seen", post(post_seen))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(app_id: &str, from: i64, to: i64) -> RankMove {
        RankMove {
            app_id: app_id.to_string(),
            app_slug: format!("slug-{app_id}"),
            app_name: format!("Name {app_id}"),
            app_icon_url: None,
            from,
            to,
        }
    }

    fn ids(moves: &[RankMove]) -> Vec<&str> {
        moves.iter().map(|m| m.app_id.as_str()).collect()
    }

    #[test]
    fn a_one_position_move_is_suppressed_as_noise() {
        assert!(select_rank_moves(vec![mv("a", 7, 6), mv("b", 6, 7)]).is_empty());
    }

    #[test]
    fn a_two_position_move_is_exactly_at_the_threshold_and_survives() {
        assert_eq!(ids(&select_rank_moves(vec![mv("a", 7, 5)])), ["a"]);
    }

    #[test]
    fn a_down_move_is_kept_not_just_an_up_move() {
        let kept = select_rank_moves(vec![mv("a", 4, 9)]);
        assert_eq!(ids(&kept), ["a"]);
        assert_eq!(kept[0].delta(), -5);
    }

    #[test]
    fn at_most_three_movers_survive_biggest_absolute_delta_first() {
        let kept = select_rank_moves(vec![
            mv("small", 10, 8),   // +2
            mv("big", 20, 5),     // +15
            mv("slide", 3, 12),   // -9
            mv("medium", 30, 26), // +4
        ]);
        assert_eq!(ids(&kept), ["big", "slide", "medium"]);
    }

    #[test]
    fn a_big_down_move_outranks_a_small_up_move() {
        let kept = select_rank_moves(vec![mv("up", 9, 6), mv("down", 2, 12)]);
        assert_eq!(ids(&kept), ["down", "up"]);
    }

    #[test]
    fn an_unmoved_app_produces_no_item() {
        assert!(select_rank_moves(vec![mv("a", 5, 5)]).is_empty());
    }

    #[test]
    fn the_badge_counts_rewards_and_rank_moves() {
        assert_eq!(badge_count(2, 1, None), 3);
    }

    #[test]
    fn the_badge_counts_the_streak_row_only_when_todays_bonus_is_unearned() {
        assert_eq!(badge_count(0, 0, Some(false)), 1);
        assert_eq!(badge_count(0, 0, Some(true)), 0);
    }

    #[test]
    fn a_shown_but_uncounted_streak_row_leaves_the_badge_at_zero() {
        // The panel still renders the streak row here — `badge_count` is only
        // about the number on the bell.
        assert_eq!(badge_count(0, 0, Some(true)), 0);
        assert_eq!(badge_count(1, 0, Some(true)), 1);
    }

    #[test]
    fn an_empty_digest_counts_zero() {
        assert_eq!(badge_count(0, 0, None), 0);
    }

    #[test]
    fn items_serialize_with_the_dto_contracts_tagged_camel_case_shape() {
        let json = serde_json::to_value(vec![
            DigestItem::Reward {
                app_id: "app1".into(),
                app_slug: "jupiter".into(),
                app_name: "Jupiter".into(),
                app_icon_url: Some("https://example.com/i.png".into()),
                amount: 12.5,
                epoch_count: 2,
            },
            DigestItem::RankMove {
                app_id: "app2".into(),
                app_slug: "drift".into(),
                app_name: "Drift".into(),
                app_icon_url: None,
                from: 7,
                to: 4,
                delta: 3,
            },
            DigestItem::Streak {
                streak_days: 5,
                best_days: 9,
                bonus_claimed_today: false,
            },
        ])
        .expect("items serialize");

        assert_eq!(
            json,
            serde_json::json!([
                { "kind": "reward", "appId": "app1", "appSlug": "jupiter", "appName": "Jupiter",
                  "appIconUrl": "https://example.com/i.png", "amount": 12.5, "epochCount": 2 },
                { "kind": "rank_move", "appId": "app2", "appSlug": "drift", "appName": "Drift",
                  "appIconUrl": null, "from": 7, "to": 4, "delta": 3 },
                { "kind": "streak", "streakDays": 5, "bestDays": 9, "bonusClaimedToday": false },
            ])
        );
    }
}
