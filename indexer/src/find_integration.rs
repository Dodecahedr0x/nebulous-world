//! The `/find` funnel against a **live Postgres** — the tier A77 said this run
//! did not have, built after the user overrode A77.
//!
//! Everything here executes SQL. Migration 009, the 17-column `"App"` SELECT,
//! the `"AppTag" JOIN "Tag"`, `gen_random_uuid()::text`, `AVG(...)::double
//! precision` and the `ON CONFLICT` targets in `find/store.rs` had never once
//! run before this module existed: `cargo check` treats a query string as
//! opaque, so it compiles a query naming a column that does not exist (A72).
//!
//! Run it:
//!
//! ```text
//! bash app/scripts/find-test-db.sh
//! cd indexer && FIND_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres \
//!   cargo test -p nebulous-world-indexer -- --ignored
//! ```
//!
//! Every test is `#[ignore]`d, so `cargo test` with no flags is unchanged and
//! needs no database. `FIND_TEST_DATABASE_URL` is an **admin** url: each test
//! creates its own `find_it_<unique>` database, migrates it, and drops it, so
//! tests share nothing and may run in parallel. A test that panics leaks its
//! database — `find-test-db.sh` sweeps the `find_it_%` prefix on startup.
//!
//! It lives inside the bin crate rather than `indexer/tests/` because there is
//! no `[lib]` target to link against (A84).
//!
//! `TestDb`, `router_for`, `send`/`post` and `insert_app` are `pub(crate)` so
//! later live-database tiers reuse this harness (same env var, same
//! create-migrate-drop-per-test discipline, same sweep) instead of growing a
//! second one — see `digest_integration.rs`.
//!
//! This module never modifies `find/**`, `handlers/find.rs` or migration 009;
//! it only executes them. The guard-mutation battery that temporarily edits
//! those files and restores them lives outside the source tree, in the build
//! run directory (`.agent/1.13.1-guard-battery.py`).

use crate::api::{ApiError, ApiState};
use crate::find::{store, Answer, AnswerValue, FacetKind, FacetRef};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tower::util::ServiceExt;

/// The common prefix `find-test-db.sh` sweeps. A `Drop` impl cannot await, so a
/// failing test cannot drop its own database; the sweep is what keeps that from
/// accumulating.
const DB_PREFIX: &str = "find_it_";

const SETUP_HINT: &str = "\
set FIND_TEST_DATABASE_URL to an ADMIN Postgres url and run this tier explicitly:

  bash app/scripts/find-test-db.sh
  cd indexer && FIND_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres \\
    cargo test -p nebulous-world-indexer -- --ignored
";

/// Panics rather than returning early when the variable is missing. A helper
/// that skipped instead would make every test in this file pass vacuously the
/// moment the database went away — the exact inert-test failure A74 records.
fn admin_url() -> String {
    match std::env::var("FIND_TEST_DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => panic!("FIND_TEST_DATABASE_URL is not set.\n\n{SETUP_HINT}"),
    }
}

fn unique_db_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the unix epoch")
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{DB_PREFIX}{}_{nanos}_{seq}", std::process::id())
}

/// Swaps the database name in a connection url, preserving any query string
/// (`?sslmode=…`), which sits after the path and would otherwise be swallowed.
fn database_url_for(admin_url: &str, db_name: &str) -> String {
    let (base, query) = match admin_url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (admin_url, None),
    };
    let prefix = base
        .rsplit_once('/')
        .unwrap_or_else(|| panic!("FIND_TEST_DATABASE_URL has no database path: {base}"))
        .0;
    match query {
        Some(query) => format!("{prefix}/{db_name}?{query}"),
        None => format!("{prefix}/{db_name}"),
    }
}

/// A freshly created, freshly migrated database, owned by one test.
pub(crate) struct TestDb {
    name: String,
    admin: PgPool,
    pub(crate) pool: PgPool,
}

impl TestDb {
    pub(crate) async fn create() -> TestDb {
        let admin_url = admin_url();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .unwrap_or_else(|e| panic!("cannot reach {admin_url}: {e}\n\n{SETUP_HINT}"));

        let name = unique_db_name();
        sqlx::query(&format!(r#"CREATE DATABASE "{name}""#))
            .execute(&admin)
            .await
            .unwrap_or_else(|e| panic!("CREATE DATABASE {name} failed: {e}"));

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url_for(&admin_url, &name))
            .await
            .unwrap_or_else(|e| panic!("cannot connect to {name}: {e}"));

        // The same macro call `db.rs` makes at startup, resolved against
        // CARGO_MANIFEST_DIR — executing 009 through the real path is half the
        // point of this tier, so it is never hand-applied with psql.
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("migrations failed on {name}: {e}"));

        TestDb { name, admin, pool }
    }

    pub(crate) async fn destroy(self) {
        self.pool.close().await;
        let _ = sqlx::query(&format!(
            r#"DROP DATABASE IF EXISTS "{}" WITH (FORCE)"#,
            self.name
        ))
        .execute(&self.admin)
        .await;
        self.admin.close().await;
    }
}

/// `ApiError` is deliberately not `Debug` (it is a response type, not a
/// diagnostic), so `.unwrap()` will not compile on these.
fn unwrap_api<T>(result: Result<T, ApiError>, what: &str) -> T {
    match result {
        Ok(value) => value,
        Err(err) => panic!("{what} failed: {} {}", err.0, err.1),
    }
}

/// The FULL api router, not `handlers::find::routes()` — so the
/// `.merge(crate::handlers::find::routes())` line in `api.rs` is under test too.
/// `RpcClient::new_with_commitment` opens no connection, so the dummy url costs
/// nothing; no test here reaches an RPC or DLMM path.
pub(crate) fn router_for(pool: PgPool) -> Router {
    crate::api::router(Arc::new(ApiState {
        pool,
        rpc: RpcClient::new_with_commitment(
            "http://127.0.0.1:1".to_string(),
            CommitmentConfig::confirmed(),
        ),
        http: reqwest::Client::new(),
        program_id: Pubkey::default(),
        vote_token_mint: Pubkey::default(),
        admin_authority: Pubkey::default(),
        dlmm_bridge_url: "http://127.0.0.1:1".to_string(),
    }))
}

pub(crate) async fn send(
    router: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(match &body {
            Some(value) => Body::from(value.to_string()),
            None => Body::empty(),
        })
        .expect("well-formed test request");

    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("axum Router is infallible");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

pub(crate) async fn post(router: &Router, path: &str, body: Value) -> (StatusCode, Value) {
    send(router, "POST", path, Some(body)).await
}

/// `"App"."updatedAt"` is the one NOT NULL column with no default, so every
/// insert must supply it.
pub(crate) async fn insert_app(pool: &PgPool, id: &str, status: &str) {
    sqlx::query(
        r#"INSERT INTO "App" (id, slug, name, url, status, "updatedAt")
           VALUES ($1, $2, $3, 'https://example.com', $4, now())"#,
    )
    .bind(id)
    .bind(format!("slug-{id}"))
    .bind(format!("Name {id}"))
    .bind(status)
    .execute(pool)
    .await
    .expect("App insert");
}

fn answer(kind: FacetKind, value: &str, response: AnswerValue) -> Answer {
    Answer {
        facet: FacetRef {
            kind,
            value: value.to_string(),
        },
        value: response,
    }
}

async fn facet_stat(pool: &PgPool, kind: &str, value: &str) -> (i32, i32, i32) {
    sqlx::query_as(
        r#"SELECT "yesCount", "noCount", "skipCount" FROM "FindFacetStat"
           WHERE "facetKind" = $1 AND "facetValue" = $2"#,
    )
    .bind(kind)
    .bind(value)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("no FindFacetStat row for {kind}/{value}: {e}"))
}

async fn session_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(r#"SELECT COUNT(*) FROM "FindSession""#)
        .fetch_one(pool)
        .await
        .expect("FindSession count")
}

// ---------------------------------------------------------------------
// A77 (a) — a replayed confirm must not inflate the facet tallies
// ---------------------------------------------------------------------

/// The `if inserted { bump_facet_stats }` guard in `handlers::find::confirm`,
/// end to end through the router. It rests on three things that only a database
/// can settle: the unique index in 009, `ON CONFLICT … DO NOTHING`'s empty
/// `RETURNING`, and the handler reading that as "already recorded".
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn a_replayed_confirm_does_not_inflate_the_facet_tallies_a77a() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());

    let body = json!({
        "answers": [
            { "facet": { "kind": "tag", "value": "lending" }, "value": "yes" },
            { "facet": { "kind": "category", "value": "defi" }, "value": "no" },
            { "facet": { "kind": "chain", "value": "base" }, "value": "skip" }
        ],
        "appId": "app-1",
        "outcome": "confirmed",
        "visitorId": "v-1",
        "sessionId": "s-1",
        "turnstileVerified": true
    });

    for attempt in 1..=2 {
        let (status, value) = post(&router, "/find/confirm", body.clone()).await;
        assert_eq!(status, StatusCode::OK, "attempt {attempt}: {value}");
        assert_eq!(value, json!({ "ok": true }), "attempt {attempt}");
        // Asserted after BOTH attempts, not only at the end: "1 after the
        // first" and "1 after the second" are different claims, and checking
        // only the total cannot tell `if inserted` apart from `if !inserted`
        // — which bumps exactly once too, on the replay instead of the write.
        assert_eq!(
            facet_stat(&db.pool, "tag", "lending").await,
            (1, 0, 0),
            "after attempt {attempt}"
        );
    }

    assert_eq!(
        session_count(&db.pool).await,
        1,
        "the replay inserted a row"
    );

    // The `SqlxJson(answers)` bind landing in the JSONB column, and
    // `questionsAsked` coming from the answer history rather than a literal.
    let (stored, questions_asked): (sqlx::types::Json<Vec<Answer>>, i32) =
        sqlx::query_as(r#"SELECT "answers", "questionsAsked" FROM "FindSession""#)
            .fetch_one(&db.pool)
            .await
            .expect("stored session row");
    assert_eq!(questions_asked, 3);
    assert_eq!(stored.0.len(), 3, "{:?}", stored.0);
    assert_eq!(stored.0[0].facet.kind, FacetKind::Tag);
    assert_eq!(stored.0[0].facet.value, "lending");
    assert_eq!(stored.0[0].value, AnswerValue::Yes);

    assert_eq!(facet_stat(&db.pool, "tag", "lending").await, (1, 0, 0));
    assert_eq!(facet_stat(&db.pool, "category", "defi").await, (0, 1, 0));
    assert_eq!(facet_stat(&db.pool, "chain", "base").await, (0, 0, 1));

    db.destroy().await;
}

// ---------------------------------------------------------------------
// A77 (b) — WHERE status = 'approved'
// ---------------------------------------------------------------------

/// The funnel must never score a pending or rejected app. Both halves matter:
/// `candidateCount` is what a mid-funnel turn discloses, and the shortlist is
/// what a finished one returns.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn only_approved_apps_are_candidates_a77b() {
    let db = TestDb::create().await;
    insert_app(&db.pool, "app-approved", "approved").await;
    insert_app(&db.pool, "app-pending", "pending").await;
    insert_app(&db.pool, "app-rejected", "rejected").await;
    let router = router_for(db.pool.clone());

    let (status, value) = post(&router, "/find/next", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["candidateCount"], 1, "{value}");

    let (status, value) = post(&router, "/find/next", json!({ "forceResults": true })).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let shortlist = value["shortlist"].as_array().expect("shortlist array");
    let ids: Vec<&str> = shortlist
        .iter()
        .map(|entry| entry["app"]["id"].as_str().expect("app id"))
        .collect();
    assert_eq!(ids, ["app-approved"], "{value}");

    db.destroy().await;
}

// ---------------------------------------------------------------------
// A77 (c) — the three route path strings
// ---------------------------------------------------------------------

/// A typo in a path string only ever surfaced at runtime before this. The 404
/// case is what stops the test passing against a router that matches
/// everything.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn the_three_find_routes_are_mounted_and_a_near_miss_is_not_a77c() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());

    let confirm = json!({
        "answers": [],
        "appId": "app-1",
        "outcome": "confirmed",
        "visitorId": "v-1",
        "sessionId": "s-1",
        "turnstileVerified": true
    });

    for (method, path, body) in [
        ("POST", "/find/next", Some(json!({}))),
        ("POST", "/find/confirm", Some(confirm)),
        ("GET", "/find/stats", None),
    ] {
        let (status, value) = send(&router, method, path, body).await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} is not mounted"
        );
        assert_eq!(status, StatusCode::OK, "{method} {path} -> {value}");
    }

    let (status, _) = post(&router, "/find/nex", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "/find/nex must not match");

    db.destroy().await;
}

// ---------------------------------------------------------------------
// store.rs round-trips
// ---------------------------------------------------------------------

/// The idempotency key is the TRIPLE `("sessionId", "appId", "outcome")`, not
/// the pair: a visitor who rejects then confirms the same app in one session
/// has said two different things, and both are training signal.
///
/// Also the one execution of `gen_random_uuid()::text`, whose result is the
/// primary key — a constant there would make the second insert fail outright.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn record_outcome_is_idempotent_per_session_app_outcome_triple() {
    let db = TestDb::create().await;
    let record = |outcome: &'static str| {
        let pool = db.pool.clone();
        async move { store::record_outcome(&pool, "v-1", "s-1", "app-1", outcome, &[], 2, true).await }
    };

    assert!(
        unwrap_api(record("confirmed").await, "first confirmed"),
        "the first write must insert"
    );
    assert!(
        !unwrap_api(record("confirmed").await, "replayed confirmed"),
        "a replay of the same triple must be a no-op"
    );
    assert!(
        unwrap_api(record("rejected").await, "same session, other outcome"),
        "varying only the outcome must insert a second row"
    );

    assert_eq!(session_count(&db.pool).await, 2);

    let ids: Vec<String> = sqlx::query_scalar(r#"SELECT id FROM "FindSession""#)
        .fetch_all(&db.pool)
        .await
        .expect("session ids");
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1], "gen_random_uuid() produced a duplicate id");
    for id in &ids {
        assert_eq!(id.len(), 36, "not a uuid text: {id:?}");
    }

    db.destroy().await;
}

/// A80: an outcome whose Turnstile token failed is *recorded* but never
/// trained on. Both halves are asserted, because a `WHERE` that dropped the row
/// at write time would satisfy the learned-map assertion on its own.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn load_learned_aggregates_verified_rows_and_ignores_unverified_ones_a80() {
    let db = TestDb::create().await;
    let write = |session: &str, app: &str, outcome: &str, verified: bool| {
        let (pool, session, app, outcome) = (
            db.pool.clone(),
            session.to_string(),
            app.to_string(),
            outcome.to_string(),
        );
        async move {
            unwrap_api(
                store::record_outcome(&pool, "v-1", &session, &app, &outcome, &[], 3, verified)
                    .await,
                "record_outcome",
            )
        }
    };

    assert!(write("s-1", "app-good", "confirmed", true).await);
    assert!(write("s-2", "app-bad", "rejected", true).await);
    assert!(write("s-3", "app-unverified", "confirmed", false).await);

    // The unverified row IS in the table — A80 replaced A29's refused write
    // precisely so the Turnstile loss stays countable.
    assert_eq!(session_count(&db.pool).await, 3);
    let unverified: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(*) FROM "FindSession" WHERE NOT "turnstileVerified""#)
            .fetch_one(&db.pool)
            .await
            .expect("unverified count");
    assert_eq!(unverified, 1);

    let learned = unwrap_api(store::load_learned(&db.pool).await, "load_learned");
    assert_eq!(learned.get("app-good"), Some(&(1.0, 1.0)));
    assert_eq!(learned.get("app-bad"), Some(&(1.0, 0.0)));
    assert_eq!(
        learned.get("app-unverified"),
        None,
        "an unverified outcome reached the learned weights: {learned:?}"
    );

    db.destroy().await;
}

/// Criterion 7's metric, and the one execution of the `::double precision`
/// cast — without it `AVG` over an `INTEGER` column comes back `NUMERIC` and
/// sqlx refuses to decode it as `f64`.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn avg_questions_to_confirm_counts_only_confirmed_sessions() {
    let db = TestDb::create().await;
    let avg = || store::avg_questions_to_confirm(&db.pool);

    assert_eq!(
        unwrap_api(avg().await, "avg on an empty table"),
        None,
        "no confirmed session yet must read as None, not 0"
    );

    for (session, questions) in [("s-1", 3), ("s-2", 5)] {
        assert!(unwrap_api(
            store::record_outcome(
                &db.pool,
                "v-1",
                session,
                "app-1",
                "confirmed",
                &[],
                questions,
                true
            )
            .await,
            "confirmed insert",
        ));
    }
    assert_eq!(unwrap_api(avg().await, "avg over two confirms"), Some(4.0));

    assert!(unwrap_api(
        store::record_outcome(&db.pool, "v-1", "s-3", "app-1", "rejected", &[], 100, true).await,
        "rejected insert",
    ));
    assert_eq!(
        unwrap_api(avg().await, "avg with a rejected outlier"),
        Some(4.0),
        "a rejected session moved the confirm average"
    );

    db.destroy().await;
}

/// The `ON CONFLICT … DO UPDATE` arithmetic: counts accumulate across calls,
/// and the three answer values land in three different columns.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn bump_facet_stats_accumulates_across_calls() {
    let db = TestDb::create().await;
    let lending = answer(FacetKind::Tag, "lending", AnswerValue::Yes);

    for _ in 0..2 {
        unwrap_api(
            store::bump_facet_stats(&db.pool, std::slice::from_ref(&lending)).await,
            "bump yes",
        );
    }
    assert_eq!(facet_stat(&db.pool, "tag", "lending").await, (2, 0, 0));

    unwrap_api(
        store::bump_facet_stats(
            &db.pool,
            &[answer(FacetKind::Tag, "lending", AnswerValue::Skip)],
        )
        .await,
        "bump skip",
    );
    assert_eq!(
        facet_stat(&db.pool, "tag", "lending").await,
        (2, 0, 1),
        "a Skip overwrote the Yes tally instead of landing in its own column"
    );

    db.destroy().await;
}

// ---------------------------------------------------------------------
// handlers/find.rs — the 17-column SELECT and the AppTag/Tag join (A72)
// ---------------------------------------------------------------------

/// The query A72 flagged: 17 `"App"` columns and an `"AppTag" JOIN "Tag"`,
/// verified until now only by reading them against `005_app_schema.sql`. Every
/// column is given a non-default value so a misspelled identifier or a
/// mis-ordered `FromRow` cannot pass; `status` is the one exception, because
/// its default *is* `'approved'` and the row has to be approved to be selected.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn fetch_approved_apps_reads_every_column_and_joins_tags_a72() {
    let db = TestDb::create().await;

    // `"App"."submittedBy"` and `"AppTag"."suggestedBy"` are both FKs onto
    // `"User"` — two constraints that only execution can reveal, since nothing
    // in the query text mentions them.
    for (id, wallet) in [
        ("user-sub", "SubmitterPubkey"),
        ("user-sug", "SuggesterPubkey"),
    ] {
        sqlx::query(r#"INSERT INTO "User" (id, wallet, "updatedAt") VALUES ($1, $2, now())"#)
            .bind(id)
            .bind(wallet)
            .execute(&db.pool)
            .await
            .expect("User insert");
    }

    sqlx::query(
        r#"INSERT INTO "App" (id, slug, name, tagline, description, url, "iconUrl",
                              category, chain, status, "submittedBy", "createdAt",
                              "updatedAt", "voteCount", "voteWeight", "stakeTotal",
                              "viewCount", "rankScore")
           VALUES ('app-1', 'aster', 'Aster', 'A tagline', 'A description',
                   'https://aster.example', 'https://cdn.example/icon.png',
                   'gaming', 'base', 'approved', 'user-sub',
                   TIMESTAMP '2026-03-04 05:06:07', now(),
                   7, 12.5, 33.25, 99, 4.5)"#,
    )
    .execute(&db.pool)
    .await
    .expect("App insert");

    for (tag_id, slug, name) in [
        ("tag-lend", "lending", "Lending"),
        ("tag-dex", "dex", "DEX"),
    ] {
        sqlx::query(r#"INSERT INTO "Tag" (id, slug, name) VALUES ($1, $2, $3)"#)
            .bind(tag_id)
            .bind(slug)
            .bind(name)
            .execute(&db.pool)
            .await
            .expect("Tag insert");
    }
    // Different stakes, so the `ORDER BY at."stakeTotal" DESC` in the join is
    // observable rather than incidental.
    for (id, tag_id, stake) in [("at-lend", "tag-lend", 10.0), ("at-dex", "tag-dex", 90.0)] {
        sqlx::query(
            r#"INSERT INTO "AppTag" (id, "appId", "tagId", "suggestedBy", "stakeTotal")
               VALUES ($1, 'app-1', $2, 'user-sug', $3)"#,
        )
        .bind(id)
        .bind(tag_id)
        .bind(stake)
        .execute(&db.pool)
        .await
        .expect("AppTag insert");
    }

    let router = router_for(db.pool.clone());
    let (status, value) = post(&router, "/find/next", json!({ "forceResults": true })).await;
    assert_eq!(status, StatusCode::OK, "{value}");

    let app = &value["shortlist"][0]["app"];
    assert_eq!(app["id"], "app-1", "{value}");
    assert_eq!(app["slug"], "aster");
    assert_eq!(app["name"], "Aster");
    assert_eq!(app["tagline"], "A tagline");
    assert_eq!(app["description"], "A description");
    assert_eq!(app["url"], "https://aster.example");
    assert_eq!(app["iconUrl"], "https://cdn.example/icon.png");
    assert_eq!(app["category"], "gaming");
    assert_eq!(app["chain"], "base");
    assert_eq!(app["status"], "approved");
    assert_eq!(app["createdAt"], "2026-03-04T05:06:07+00:00");
    assert_eq!(app["submittedBy"], "user-sub");
    assert_eq!(app["voteCount"], 7);
    assert_eq!(app["voteWeight"], 12.5);
    assert_eq!(app["stakeTotal"], 33.25);
    assert_eq!(app["viewCount"], 99);
    assert_eq!(app["rankScore"], 4.5);

    let tags = app["tags"].as_array().expect("tags array");
    let slugs: Vec<&str> = tags
        .iter()
        .map(|tag| tag["slug"].as_str().expect("tag slug"))
        .collect();
    assert_eq!(slugs, ["dex", "lending"], "tag join or its order is wrong");
    assert_eq!(tags[0]["id"], "at-dex");
    assert_eq!(tags[0]["tagId"], "tag-dex");
    assert_eq!(tags[0]["name"], "DEX");
    assert_eq!(tags[0]["stakeTotal"], 90.0);
    assert_eq!(tags[0]["suggestedBy"], "user-sug");

    db.destroy().await;
}

/// `GET /find/stats` end to end, so the `Option<f64>` round-trips through the
/// handler and out as JSON rather than only through `store`.
#[tokio::test]
#[ignore = "needs a live Postgres — see app/scripts/find-test-db.sh"]
async fn the_stats_route_reports_the_average_end_to_end() {
    let db = TestDb::create().await;
    let router = router_for(db.pool.clone());

    let (status, value) = send(&router, "GET", "/find/stats", None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value, json!({ "avgQuestionsToConfirm": null }));

    for (session, questions) in [("s-1", 2), ("s-2", 6)] {
        assert!(unwrap_api(
            store::record_outcome(
                &db.pool,
                "v-1",
                session,
                "app-1",
                "confirmed",
                &[],
                questions,
                true
            )
            .await,
            "confirmed insert",
        ));
    }

    let (status, value) = send(&router, "GET", "/find/stats", None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value, json!({ "avgQuestionsToConfirm": 4.0 }));

    db.destroy().await;
}
