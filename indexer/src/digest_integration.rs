//! The staker digest against a **live Postgres**, the same tier (and the same
//! harness — see `find_integration.rs`, whose `TestDb`/`router_for`/`send`
//! this reuses) that `/find` gets.
//!
//! It exists for the same reason: `cargo check` treats a query string as
//! opaque, so every column name, the `RANK() OVER` windows, the gaps-and-
//! islands backfill in migration 010 and the streak `UPDATE`'s CASE/GREATEST
//! arithmetic compile fine while being wrong. Only a database settles them.
//!
//! Run it:
//!
//! ```text
//! bash app/scripts/find-test-db.sh
//! cd indexer && FIND_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres \
//!   cargo test -p nebulous-world-indexer -- --ignored
//! ```
//!
//! Every test is `#[ignore]`d, so `cargo test` with no flags still needs no
//! database. The pure halves of the same logic (the rank-move noise guards,
//! the badge count, the streak transition table) are unit-tested without one
//! in `handlers/digest.rs` and `handlers/xp.rs`.

use crate::find_integration::{insert_app, router_for, send, TestDb};
use axum::http::StatusCode;
use axum::Router;
use chrono::NaiveDate;
use serde_json::{json, Value};
use sqlx::PgPool;

fn today() -> NaiveDate {
    chrono::Utc::now().naive_utc().date()
}

fn days_ago(n: i64) -> NaiveDate {
    today() - chrono::Duration::days(n)
}

async fn insert_user(pool: &PgPool, id: &str) {
    sqlx::query(
        r#"INSERT INTO "User" (id, wallet, "updatedAt") VALUES ($1, $2, now())"#,
    )
    .bind(id)
    .bind(format!("wallet-{id}"))
    .execute(pool)
    .await
    .expect("User insert");
}

/// The digest's own three columns plus `lastXpDate`, set in one place so each
/// test states its starting position in one line.
async fn set_user_state(
    pool: &PgPool,
    id: &str,
    last_xp_date: Option<NaiveDate>,
    streak_days: i32,
    best_days: i32,
) {
    sqlx::query(
        r#"UPDATE "User" SET "lastXpDate" = $2, "streakDays" = $3, "streakBestDays" = $4 WHERE id = $1"#,
    )
    .bind(id)
    .bind(last_xp_date)
    .bind(streak_days)
    .bind(best_days)
    .execute(pool)
    .await
    .expect("User state update");
}

async fn read_streak(pool: &PgPool, id: &str) -> (Option<NaiveDate>, i32, i32) {
    sqlx::query_as(r#"SELECT "lastXpDate", "streakDays", "streakBestDays" FROM "User" WHERE id = $1"#)
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("User streak read")
}

/// An active stake by `user_id` on `app_id` — the membership test both the
/// reward and the rank-move halves gate on. Each call makes its own
/// `Tag`/`AppTag` so callers never collide.
async fn stake_on(pool: &PgPool, user_id: &str, app_id: &str) {
    let tag_id = format!("tag-{app_id}");
    let app_tag_id = format!("apptag-{app_id}-{user_id}");
    sqlx::query(r#"INSERT INTO "Tag" (id, slug, name) VALUES ($1, $1, $1) ON CONFLICT DO NOTHING"#)
        .bind(&tag_id)
        .execute(pool)
        .await
        .expect("Tag insert");
    sqlx::query(r#"INSERT INTO "AppTag" (id, "appId", "tagId") VALUES ($1, $2, $3)"#)
        .bind(&app_tag_id)
        .bind(app_id)
        .bind(&tag_id)
        .execute(pool)
        .await
        .expect("AppTag insert");
    sqlx::query(r#"INSERT INTO "Stake" (id, "appTagId", "userId", amount, active) VALUES ($1, $2, $3, 100, true)"#)
        .bind(format!("stake-{app_tag_id}"))
        .bind(&app_tag_id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("Stake insert");
}

/// A settled epoch on `app_id` plus this user's share of it — the shape
/// `handlers/revenue.rs::settle_epoch` leaves behind (one `RevenueClaim` per
/// participant, `claimed = false` until they actually claim it).
async fn settled_epoch(pool: &PgPool, id: &str, app_id: &str, user_id: &str, amount: f64, claimed: bool) {
    sqlx::query(
        r#"INSERT INTO "RevenueEpoch" (id, "appId", "periodStart", "periodEnd", "grossRevenue", distributed, "closedAt")
           VALUES ($1, $2, now() - interval '7 days', now(), 999, true, now())"#,
    )
    .bind(id)
    .bind(app_id)
    .execute(pool)
    .await
    .expect("RevenueEpoch insert");
    sqlx::query(
        r#"INSERT INTO "RevenueClaim" (id, "epochId", "userId", amount, claimed) VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(format!("claim-{id}-{user_id}"))
    .bind(id)
    .bind(user_id)
    .bind(amount)
    .bind(claimed)
    .execute(pool)
    .await
    .expect("RevenueClaim insert");
}

async fn set_rank_score(pool: &PgPool, app_id: &str, rank_score: f64) {
    sqlx::query(r#"UPDATE "App" SET "rankScore" = $2 WHERE id = $1"#)
        .bind(app_id)
        .bind(rank_score)
        .execute(pool)
        .await
        .expect("App rankScore update");
}

/// `date_trunc('day', ...)` exactly like `revenue.rs::write_daily_snapshot`
/// does — one shared timestamp per snapshot day. Without the truncation each
/// insert would land on its own microsecond, and `MAX("date")` would pick a
/// baseline containing exactly one app, silently ranking it 1st.
async fn snapshot(pool: &PgPool, app_id: &str, days_ago: i64, rank_score: f64) {
    sqlx::query(
        r#"INSERT INTO "AppStatsSnapshot" (id, "appId", date, "voteWeight", "stakeTotal", "viewCount", "rankScore")
           VALUES (gen_random_uuid()::text, $1, date_trunc('day', now() - ($2 || ' days')::interval), 0, 0, 0, $3)"#,
    )
    .bind(app_id)
    .bind(days_ago.to_string())
    .bind(rank_score)
    .execute(pool)
    .await
    .expect("AppStatsSnapshot insert");
}

async fn set_seen_at(pool: &PgPool, user_id: &str, days_ago: Option<i64>) {
    match days_ago {
        Some(n) => {
            sqlx::query(
                r#"UPDATE "User" SET "digestSeenAt" = now() - ($2 || ' days')::interval WHERE id = $1"#,
            )
            .bind(user_id)
            .bind(n.to_string())
            .execute(pool)
            .await
            .expect("digestSeenAt update");
        }
        None => {
            sqlx::query(r#"UPDATE "User" SET "digestSeenAt" = NULL WHERE id = $1"#)
                .bind(user_id)
                .execute(pool)
                .await
                .expect("digestSeenAt clear");
        }
    }
}

async fn digest(router: &Router, user_id: &str) -> Value {
    let (status, body) = send(router, "GET", &format!("/digest?userId={user_id}"), None).await;
    assert_eq!(status, StatusCode::OK, "GET /digest failed: {body}");
    body
}

fn items_of_kind<'a>(body: &'a Value, kind: &str) -> Vec<&'a Value> {
    body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .filter(|item| item["kind"] == kind)
        .collect()
}

// ---------------------------------------------------------------------
// Streak transitions — through the real `award`, i.e. the real UPDATE.
// ---------------------------------------------------------------------

/// One `award` call against a user in a known starting position, returning
/// where the streak landed.
async fn award_once(pool: &PgPool, user_id: &str, kind: &str) {
    crate::handlers::xp::award(pool, user_id, kind, Some("target"), 20)
        .await
        .expect("award");
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_first_ever_qualifying_action_starts_the_streak_at_one() {
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;

    award_once(&db.pool, "u1", "vote").await;

    assert_eq!(read_streak(&db.pool, "u1").await, (Some(today()), 1, 1));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_consecutive_day_extends_the_streak_and_leaves_the_best_alone() {
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(1)), 4, 9).await;

    award_once(&db.pool, "u1", "vote").await;

    assert_eq!(read_streak(&db.pool, "u1").await, (Some(today()), 5, 9));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_gap_of_one_missed_day_restarts_the_streak() {
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(2)), 4, 9).await;

    award_once(&db.pool, "u1", "vote").await;

    assert_eq!(read_streak(&db.pool, "u1").await, (Some(today()), 1, 9));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_gap_of_many_days_restarts_the_streak_but_keeps_the_best() {
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(90)), 40, 40).await;

    award_once(&db.pool, "u1", "vote").await;

    assert_eq!(read_streak(&db.pool, "u1").await, (Some(today()), 1, 40));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn the_best_ratchets_against_todays_count_not_yesterdays() {
    // The GREATEST must see 10, not the 9 a bare `"streakDays"` reference
    // would read (old-value semantics inside one UPDATE).
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(1)), 9, 9).await;

    award_once(&db.pool, "u1", "vote").await;

    assert_eq!(read_streak(&db.pool, "u1").await, (Some(today()), 10, 10));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_second_qualifying_action_the_same_day_does_not_advance_the_streak() {
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(1)), 4, 9).await;

    award_once(&db.pool, "u1", "vote").await;
    award_once(&db.pool, "u1", "stake").await;

    assert_eq!(read_streak(&db.pool, "u1").await, (Some(today()), 5, 9));
    let bonuses: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM "XpEvent" WHERE "userId" = 'u1' AND kind = 'daily_bonus'"#,
    )
    .fetch_one(&db.pool)
    .await
    .expect("daily_bonus count");
    assert_eq!(bonuses, 1, "the daily bonus is once per UTC day");
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn the_loser_of_a_same_day_race_cannot_reset_a_live_streak() {
    // Both racers read lastXpDate = yesterday and both reach the UPDATE; only
    // one of them wins the daily_bonus unique index, and the other must be
    // inert. Running the statement twice IS that race, minus the timing.
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(1)), 4, 9).await;

    crate::handlers::xp::mark_day_earned(&db.pool, "u1", today()).await.expect("first");
    crate::handlers::xp::mark_day_earned(&db.pool, "u1", today()).await.expect("second");

    assert_eq!(read_streak(&db.pool, "u1").await, (Some(today()), 5, 9));
    db.destroy().await;
}

// ---------------------------------------------------------------------
// Migration 010's backfill — the gaps-and-islands run detection.
// ---------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn the_streak_columns_exist_and_default_to_zero() {
    let db = TestDb::create().await;
    insert_user(&db.pool, "u1").await;
    assert_eq!(read_streak(&db.pool, "u1").await, (None, 0, 0));
    db.destroy().await;
}

async fn daily_bonus_on(pool: &PgPool, user_id: &str, days_ago: i64) {
    let award_date = today() - chrono::Duration::days(days_ago);
    sqlx::query(
        r#"INSERT INTO "XpEvent" (id, "userId", kind, amount, "createdAt", "awardDate")
           VALUES (gen_random_uuid()::text, $1, 'daily_bonus', 15, $2::date, $2)"#,
    )
    .bind(user_id)
    .bind(award_date)
    .execute(pool)
    .await
    .expect("XpEvent insert");
}

/// Migration 010's own text, replayed against hand-placed `daily_bonus`
/// history — `TestDb::create` applies it to an EMPTY database, so the
/// gaps-and-islands backfill (the part with all the risk in it) would
/// otherwise never execute against a single row. Replaying is safe by
/// construction: every statement in the file is idempotent.
#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn the_migration_backfills_streaks_from_existing_daily_bonus_history() {
    let db = TestDb::create().await;

    // A live run of 3 ending YESTERDAY, and an older, longer run of 5.
    insert_user(&db.pool, "live").await;
    for d in [1, 2, 3, 16, 17, 18, 19, 20] {
        daily_bonus_on(&db.pool, "live", d).await;
    }
    // A run of 4 that ended 5 days ago — already broken, so current is 0.
    insert_user(&db.pool, "broken").await;
    for d in [5, 6, 7, 8] {
        daily_bonus_on(&db.pool, "broken", d).await;
    }
    // A run of 2 that includes TODAY.
    insert_user(&db.pool, "hot").await;
    for d in [0, 1] {
        daily_bonus_on(&db.pool, "hot", d).await;
    }
    // No history at all.
    insert_user(&db.pool, "cold").await;

    sqlx::raw_sql(include_str!("../migrations/010_digest_and_streak.sql"))
        .execute(&db.pool)
        .await
        .expect("migration 010 replays cleanly");

    assert_eq!(read_streak(&db.pool, "live").await, (None, 3, 5));
    assert_eq!(read_streak(&db.pool, "broken").await, (None, 0, 4));
    assert_eq!(read_streak(&db.pool, "hot").await, (None, 2, 2));
    assert_eq!(read_streak(&db.pool, "cold").await, (None, 0, 0));

    // The watermark starts at migration time, not `createdAt` — so a
    // pre-existing user's first panel is not a wall of accumulated moves.
    let (seen_at, created_at): (Option<chrono::NaiveDateTime>, chrono::NaiveDateTime) =
        sqlx::query_as(r#"SELECT "digestSeenAt", "createdAt" FROM "User" WHERE id = 'live'"#)
            .fetch_one(&db.pool)
            .await
            .expect("watermark read");
    assert!(seen_at.expect("backfilled watermark") >= created_at);
    db.destroy().await;
}

// ---------------------------------------------------------------------
// Rewards — outstanding state, never watermark-filtered.
// ---------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn an_already_claimed_epoch_is_excluded_and_unclaimed_ones_are_summed() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    insert_app(&db.pool, "app1", "approved").await;
    stake_on(&db.pool, "u1", "app1").await;
    settled_epoch(&db.pool, "e1", "app1", "u1", 12.5, false).await;
    settled_epoch(&db.pool, "e2", "app1", "u1", 7.5, false).await;
    settled_epoch(&db.pool, "e3", "app1", "u1", 100.0, true).await;

    let body = digest(&router, "u1").await;
    let rewards = items_of_kind(&body, "reward");

    assert_eq!(rewards.len(), 1, "one row per app: {body}");
    assert_eq!(rewards[0]["appId"], json!("app1"));
    assert_eq!(rewards[0]["appSlug"], json!("slug-app1"));
    assert_eq!(rewards[0]["amount"], json!(20.0));
    assert_eq!(rewards[0]["epochCount"], json!(2));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn an_unclaimed_reward_survives_a_watermark_advance() {
    // Money is a state, not an event: opening the panel must not make it
    // disappear the way a rank move does.
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    insert_app(&db.pool, "app1", "approved").await;
    stake_on(&db.pool, "u1", "app1").await;
    settled_epoch(&db.pool, "e1", "app1", "u1", 12.5, false).await;

    assert_eq!(items_of_kind(&digest(&router, "u1").await, "reward").len(), 1);

    let (status, seen) = send(&router, "POST", "/digest/seen", Some(json!({ "userId": "u1" }))).await;
    assert_eq!(status, StatusCode::OK, "POST /digest/seen failed: {seen}");
    assert!(seen["seenAt"].is_string(), "the new watermark comes back: {seen}");

    let body = digest(&router, "u1").await;
    let rewards = items_of_kind(&body, "reward");
    assert_eq!(rewards.len(), 1, "still outstanding after the advance: {body}");
    assert_eq!(rewards[0]["amount"], json!(12.5));
    assert_eq!(body["seenAt"], seen["seenAt"], "GET reports the advanced watermark");
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn an_undistributed_epoch_is_not_claimable_yet() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    insert_app(&db.pool, "app1", "approved").await;
    stake_on(&db.pool, "u1", "app1").await;
    settled_epoch(&db.pool, "e1", "app1", "u1", 12.5, false).await;
    sqlx::query(r#"UPDATE "RevenueEpoch" SET distributed = false WHERE id = 'e1'"#)
        .execute(&db.pool)
        .await
        .expect("undistribute");

    assert!(items_of_kind(&digest(&router, "u1").await, "reward").is_empty());
    db.destroy().await;
}

// ---------------------------------------------------------------------
// Rank moves — watermark-filtered, ±2 threshold, top 3, down-moves shown.
// ---------------------------------------------------------------------

/// `n` apps, ranked 1..n right now and 1..n reversed at the baseline, so each
/// test only has to say which apps it wants and how far they moved.
async fn app_ranked(pool: &PgPool, user_id: &str, app_id: &str, now_score: f64) {
    insert_app(pool, app_id, "approved").await;
    stake_on(pool, user_id, app_id).await;
    set_rank_score(pool, app_id, now_score).await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_null_watermark_emits_no_rank_moves() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_seen_at(&db.pool, "u1", None).await;
    app_ranked(&db.pool, "u1", "app1", 10.0).await;
    app_ranked(&db.pool, "u1", "app2", 20.0).await;
    // A perfectly good baseline exists — the missing watermark is what
    // suppresses these, not missing data.
    snapshot(&db.pool, "app1", 1, 90.0).await;
    snapshot(&db.pool, "app2", 1, 1.0).await;

    let body = digest(&router, "u1").await;
    assert!(items_of_kind(&body, "rank_move").is_empty(), "{body}");
    assert_eq!(body["seenAt"], Value::Null);
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn no_snapshot_at_or_before_the_watermark_emits_no_rank_moves() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_seen_at(&db.pool, "u1", Some(5)).await;
    app_ranked(&db.pool, "u1", "app1", 10.0).await;
    app_ranked(&db.pool, "u1", "app2", 20.0).await;
    // Both snapshots are NEWER than the watermark — comparing against them
    // would be comparing the present with the present.
    snapshot(&db.pool, "app1", 1, 90.0).await;
    snapshot(&db.pool, "app2", 1, 1.0).await;

    let body = digest(&router, "u1").await;
    assert!(items_of_kind(&body, "rank_move").is_empty(), "{body}");
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_single_position_move_is_suppressed_but_a_two_position_one_is_not() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_seen_at(&db.pool, "u1", Some(1)).await;
    // Now: a=1st, b=2nd, c=3rd. Baseline: b=1st, a=2nd, c=3rd.
    app_ranked(&db.pool, "u1", "a", 30.0).await;
    app_ranked(&db.pool, "u1", "b", 20.0).await;
    app_ranked(&db.pool, "u1", "c", 10.0).await;
    snapshot(&db.pool, "a", 2, 20.0).await;
    snapshot(&db.pool, "b", 2, 30.0).await;
    snapshot(&db.pool, "c", 2, 10.0).await;

    let body = digest(&router, "u1").await;
    assert!(
        items_of_kind(&body, "rank_move").is_empty(),
        "a one-place swap is noise: {body}"
    );

    // Same setup, one more place of travel for `a` (baseline 3rd -> now 1st).
    sqlx::query(r#"UPDATE "AppStatsSnapshot" SET "rankScore" = 5 WHERE "appId" = 'a'"#)
        .execute(&db.pool)
        .await
        .expect("deepen the move");
    let body = digest(&router, "u1").await;
    let moves = items_of_kind(&body, "rank_move");
    assert_eq!(moves.len(), 1, "{body}");
    assert_eq!(moves[0]["appId"], json!("a"));
    assert_eq!(moves[0]["from"], json!(3));
    assert_eq!(moves[0]["to"], json!(1));
    assert_eq!(moves[0]["delta"], json!(2));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_down_move_is_reported_with_a_negative_delta() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_seen_at(&db.pool, "u1", Some(1)).await;
    // `faller` was 1st at the baseline and is 4th now.
    app_ranked(&db.pool, "u1", "faller", 1.0).await;
    app_ranked(&db.pool, "u1", "x", 10.0).await;
    app_ranked(&db.pool, "u1", "y", 9.0).await;
    app_ranked(&db.pool, "u1", "z", 8.0).await;
    snapshot(&db.pool, "faller", 2, 100.0).await;
    snapshot(&db.pool, "x", 2, 10.0).await;
    snapshot(&db.pool, "y", 2, 9.0).await;
    snapshot(&db.pool, "z", 2, 8.0).await;

    let body = digest(&router, "u1").await;
    let moves = items_of_kind(&body, "rank_move");
    assert_eq!(moves.len(), 1, "{body}");
    assert_eq!(moves[0]["appId"], json!("faller"));
    assert_eq!(moves[0]["from"], json!(1));
    assert_eq!(moves[0]["to"], json!(4));
    assert_eq!(moves[0]["delta"], json!(-3));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn at_most_three_rank_moves_are_reported() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_seen_at(&db.pool, "u1", Some(1)).await;
    // Five apps that exactly reverse: 1<->5, 2<->4, 3 stays.
    for (i, id) in ["a", "b", "c", "d", "e"].iter().enumerate() {
        app_ranked(&db.pool, "u1", id, (5 - i) as f64).await;
        snapshot(&db.pool, id, 2, (i + 1) as f64).await;
    }

    let body = digest(&router, "u1").await;
    let moves = items_of_kind(&body, "rank_move");
    assert_eq!(moves.len(), 3, "capped at the top three movers: {body}");
    // |delta| 4, 4, 2, 2 — the two four-place moves must both be in.
    let deltas: Vec<i64> = moves
        .iter()
        .map(|m| m["delta"].as_i64().expect("delta"))
        .collect();
    assert_eq!(deltas[0].abs(), 4);
    assert_eq!(deltas[1].abs(), 4);
    assert_eq!(deltas[2].abs(), 2);
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn an_app_the_user_does_not_stake_never_appears() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    insert_user(&db.pool, "u2").await;
    set_seen_at(&db.pool, "u1", Some(1)).await;
    // Both apps travel two places in opposite directions (a third, unstaked
    // app sits between them so neither move is sub-threshold), but only the
    // one `u1` actually stakes may show up.
    app_ranked(&db.pool, "u2", "theirs", 100.0).await;
    snapshot(&db.pool, "theirs", 2, 1.0).await;
    app_ranked(&db.pool, "u2", "filler", 50.0).await;
    snapshot(&db.pool, "filler", 2, 50.0).await;
    app_ranked(&db.pool, "u1", "mine", 1.0).await;
    snapshot(&db.pool, "mine", 2, 100.0).await;

    let body = digest(&router, "u1").await;
    let moves = items_of_kind(&body, "rank_move");
    assert_eq!(moves.len(), 1, "{body}");
    assert_eq!(moves[0]["appId"], json!("mine"));
    assert_eq!(moves[0]["delta"], json!(-2));
    db.destroy().await;
}

// ---------------------------------------------------------------------
// Badge count & the streak item, end to end.
// ---------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn the_streak_row_is_shown_but_not_counted_once_todays_bonus_is_earned() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(today()), 5, 9).await;

    let body = digest(&router, "u1").await;
    let streaks = items_of_kind(&body, "streak");
    assert_eq!(streaks.len(), 1, "{body}");
    assert_eq!(streaks[0]["streakDays"], json!(5));
    assert_eq!(streaks[0]["bestDays"], json!(9));
    assert_eq!(streaks[0]["bonusClaimedToday"], json!(true));
    assert_eq!(body["count"], json!(0), "shown, not counted: {body}");
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn the_streak_row_is_counted_while_todays_bonus_is_still_unearned() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(1)), 5, 9).await;

    let body = digest(&router, "u1").await;
    assert_eq!(items_of_kind(&body, "streak")[0]["bonusClaimedToday"], json!(false));
    assert_eq!(body["count"], json!(1), "{body}");
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn a_user_with_no_streak_state_gets_no_streak_row_and_an_empty_digest() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;

    let body = digest(&router, "u1").await;
    assert_eq!(body["items"], json!([]), "{body}");
    assert_eq!(body["count"], json!(0));
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn the_badge_sums_rewards_rank_moves_and_an_at_risk_streak() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());
    insert_user(&db.pool, "u1").await;
    set_user_state(&db.pool, "u1", Some(days_ago(1)), 5, 9).await;
    set_seen_at(&db.pool, "u1", Some(1)).await;
    app_ranked(&db.pool, "u1", "a", 30.0).await;
    app_ranked(&db.pool, "u1", "b", 20.0).await;
    app_ranked(&db.pool, "u1", "c", 10.0).await;
    snapshot(&db.pool, "a", 2, 5.0).await;
    snapshot(&db.pool, "b", 2, 30.0).await;
    snapshot(&db.pool, "c", 2, 20.0).await;
    settled_epoch(&db.pool, "e1", "a", "u1", 3.0, false).await;

    let body = digest(&router, "u1").await;
    // 1 reward + 1 rank move (a: 3rd -> 1st; b and c each moved one place)
    // + the at-risk streak.
    assert_eq!(items_of_kind(&body, "reward").len(), 1, "{body}");
    assert_eq!(items_of_kind(&body, "rank_move").len(), 1, "{body}");
    assert_eq!(body["count"], json!(3), "{body}");
    // Render order: rewards, then rank moves, then streak.
    let kinds: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, ["reward", "rank_move", "streak"]);
    db.destroy().await;
}

#[tokio::test]
#[ignore = "needs FIND_TEST_DATABASE_URL — see the module doc"]
async fn an_unknown_user_is_a_404_on_both_routes() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());

    let (status, _) = send(&router, "GET", "/digest?userId=nobody", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &router,
        "POST",
        "/digest/seen",
        Some(json!({ "userId": "nobody" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    db.destroy().await;
}
