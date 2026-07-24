//! Ports `app/src/app/api/vote/route.ts` + `vote/withdraw/route.ts`. Session
//! resolution (who's calling) stays entirely in the Next.js layer (HMAC
//! cookie, no DB — see `app/src/lib/session.ts`); these endpoints trust the
//! `userId` the caller passes, same as every on-chain-tx-building endpoint
//! in `src/api.rs` trusts the `user` pubkey it's given.

use crate::api::{ApiError, ApiState};
use crate::handlers::engine::refresh_app;
use axum::extract::{Json, Path, Query, State};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetQuery {
    app_id: String,
    user_id: String,
}

#[derive(Serialize)]
struct VoteDto {
    id: String,
    amount: f64,
}

async fn get_vote(State(state): State<Arc<ApiState>>, Query(q): Query<GetQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let vote: Option<(String, f64)> = sqlx::query_as(
        r#"SELECT id, amount FROM "Vote" WHERE "appId" = $1 AND "userId" = $2 AND active = true LIMIT 1"#,
    )
    .bind(&q.app_id)
    .bind(&q.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(Json(serde_json::json!({ "vote": vote.map(|(id, amount)| VoteDto { id, amount }) })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateReq {
    app_id: String,
    user_id: String,
    amount: f64,
    tx_sig: Option<String>,
}

async fn create(State(state): State<Arc<ApiState>>, Json(req): Json<CreateReq>) -> Result<Json<serde_json::Value>, ApiError> {
    let app_exists: bool = sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM "App" WHERE id = $1)"#)
        .bind(&req.app_id)
        .fetch_one(&state.pool)
        .await
        .map_err(crate::api::internal)?;
    if !app_exists {
        return Err(crate::api::not_found("App not found"));
    }

    if let Some(tx_sig) = &req.tx_sig {
        let existing: bool = sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM "Vote" WHERE "txSig" = $1)"#)
            .bind(tx_sig)
            .fetch_one(&state.pool)
            .await
            .map_err(crate::api::internal)?;
        if existing {
            return Err(crate::api::conflict("This transaction was already recorded"));
        }
    }

    let id: String = sqlx::query_scalar(
        r#"
        INSERT INTO "Vote" (id, "appId", "userId", amount, "txSig", "createdAt", active)
        VALUES (gen_random_uuid()::text, $1, $2, $3, $4, now(), true)
        RETURNING id
        "#,
    )
    .bind(&req.app_id)
    .bind(&req.user_id)
    .bind(req.amount)
    .bind(&req.tx_sig)
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    refresh_app(&state.pool, &req.app_id).await?;

    if let Err(e) = crate::handlers::xp::award(
        &state.pool,
        &req.user_id,
        "vote",
        Some(&req.app_id),
        crate::handlers::xp::XP_VOTE,
    )
    .await
    {
        log::warn!("failed to award vote XP for user {}: {e}", req.user_id);
    }

    let updated: (f64, i32, f64) = sqlx::query_as(
        r#"SELECT "voteWeight", "voteCount", "rankScore" FROM "App" WHERE id = $1"#,
    )
    .bind(&req.app_id)
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(Json(serde_json::json!({
        "vote": { "id": id, "amount": req.amount, "txSig": req.tx_sig },
        "app": { "voteWeight": updated.0, "voteCount": updated.1, "rankScore": updated.2 },
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawReq {
    user_id: String,
}

async fn withdraw(
    State(state): State<Arc<ApiState>>,
    Path(vote_id): Path<String>,
    Json(req): Json<WithdrawReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let row: Option<(String, String, bool)> =
        sqlx::query_as(r#"SELECT id, "userId", active FROM "Vote" WHERE id = $1"#)
            .bind(&vote_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(crate::api::internal)?;
    let Some((_, user_id, active)) = row else {
        return Err(crate::api::not_found("Vote not found"));
    };
    if user_id != req.user_id {
        return Err(crate::api::forbidden("Not your vote"));
    }
    if !active {
        return Err(crate::api::conflict("Vote already withdrawn"));
    }

    let app_id: String = sqlx::query_scalar(
        r#"UPDATE "Vote" SET active = false, "withdrawnAt" = now() WHERE id = $1 RETURNING "appId""#,
    )
    .bind(&vote_id)
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    refresh_app(&state.pool, &app_id).await?;

    Ok(Json(serde_json::json!({ "withdrawn": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawPartialReq {
    app_id: String,
    user_id: String,
    amount: f64,
}

/// Withdraws `amount` (up to the full active total) off this (user, app)'s
/// possibly-several active `Vote` rows — a user can vote on the same app
/// more than once over time (each `vote()` call just adds to the single
/// on-chain VotePosition, see programs/nebulous_world), so this consumes
/// rows oldest-first, fully deactivating each until `amount` is covered and
/// partially reducing the last one it touches. Subsumes what a former
/// `withdraw_all` did (the special case of `amount` == the full total).
/// Used by the profile page's "Your stakes" list, which sums exactly these
/// rows (see handlers/rewards.rs) — the on-chain withdraw call there always
/// withdraws this same `amount`.
async fn withdraw_partial(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<WithdrawPartialReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.amount <= 0.0 {
        return Err(crate::api::bad_request("amount must be positive"));
    }

    let rows: Vec<(String, f64)> = sqlx::query_as(
        r#"SELECT id, amount FROM "Vote" WHERE "userId" = $1 AND "appId" = $2 AND active = true ORDER BY "createdAt" ASC"#,
    )
    .bind(&req.user_id)
    .bind(&req.app_id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    // Tolerance against float drift accumulated through UI-unit conversions
    // — same magnitude as a fraction of a raw unit at this mint's decimals,
    // nowhere near enough to matter for a real withdrawal amount.
    const EPS: f64 = 1e-9;
    let mut remaining = req.amount;
    for (id, row_amount) in rows {
        if remaining <= EPS {
            break;
        }
        if row_amount <= remaining + EPS {
            sqlx::query(r#"UPDATE "Vote" SET active = false, "withdrawnAt" = now() WHERE id = $1"#)
                .bind(&id)
                .execute(&state.pool)
                .await
                .map_err(crate::api::internal)?;
            remaining -= row_amount;
        } else {
            sqlx::query(r#"UPDATE "Vote" SET amount = amount - $2 WHERE id = $1"#)
                .bind(&id)
                .bind(remaining)
                .execute(&state.pool)
                .await
                .map_err(crate::api::internal)?;
            remaining = 0.0;
        }
    }

    if remaining > EPS {
        return Err(crate::api::bad_request("amount exceeds your active vote on this app"));
    }

    refresh_app(&state.pool, &req.app_id).await?;

    Ok(Json(serde_json::json!({ "withdrawn": true })))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new()
        .route("/votes", get(get_vote).post(create))
        .route("/votes/:id/withdraw", post(withdraw))
        .route("/votes/withdraw-partial", post(withdraw_partial))
}
