//! Ports `app/src/app/api/stake/route.ts` + `stake/withdraw/route.ts`. Same
//! trust model as `votes.rs` — see that file's doc comment.

use crate::api::{not_found, ApiError, ApiState};
use crate::handlers::engine::{refresh_app, refresh_app_tag};
use axum::extract::{Json, Path, Query, State};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    app_id: String,
    user_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StakeDto {
    id: String,
    amount: f64,
    app_tag_id: String,
}

async fn list(State(state): State<Arc<ApiState>>, Query(q): Query<ListQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let rows: Vec<(String, f64, String)> = sqlx::query_as(
        r#"
        SELECT s.id, s.amount, s."appTagId"
        FROM "Stake" s JOIN "AppTag" at ON at.id = s."appTagId"
        WHERE s."userId" = $1 AND s.active = true AND at."appId" = $2
        "#,
    )
    .bind(&q.user_id)
    .bind(&q.app_id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let stakes: Vec<StakeDto> = rows.into_iter().map(|(id, amount, app_tag_id)| StakeDto { id, amount, app_tag_id }).collect();
    Ok(Json(serde_json::json!({ "stakes": stakes })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateReq {
    app_tag_id: String,
    user_id: String,
    amount: f64,
    tx_sig: Option<String>,
    simulation_mode: bool,
}

async fn create(State(state): State<Arc<ApiState>>, Json(req): Json<CreateReq>) -> Result<Json<serde_json::Value>, ApiError> {
    let app_id: Option<String> = sqlx::query_scalar(r#"SELECT "appId" FROM "AppTag" WHERE id = $1"#)
        .bind(&req.app_tag_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(crate::api::internal)?;
    let Some(app_id) = app_id else {
        return Err(not_found("Tag not found"));
    };

    if !req.simulation_mode && req.tx_sig.is_none() {
        return Err(crate::api::bad_request("A confirmed transaction signature is required"));
    }
    if let Some(tx_sig) = &req.tx_sig {
        let existing: bool = sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM "Stake" WHERE "txSig" = $1)"#)
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
        INSERT INTO "Stake" (id, "appTagId", "userId", amount, "txSig", "createdAt", active)
        VALUES (gen_random_uuid()::text, $1, $2, $3, $4, now(), true)
        RETURNING id
        "#,
    )
    .bind(&req.app_tag_id)
    .bind(&req.user_id)
    .bind(req.amount)
    .bind(&req.tx_sig)
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    refresh_app_tag(&state.pool, &req.app_tag_id).await?;
    refresh_app(&state.pool, &app_id).await?;

    if let Err(e) = crate::handlers::xp::award(
        &state.pool,
        &req.user_id,
        "stake",
        Some(&req.app_tag_id),
        crate::handlers::xp::XP_STAKE,
    )
    .await
    {
        log::warn!("failed to award stake XP for user {}: {e}", req.user_id);
    }

    Ok(Json(serde_json::json!({ "stake": { "id": id, "amount": req.amount } })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawReq {
    user_id: String,
}

async fn withdraw(
    State(state): State<Arc<ApiState>>,
    Path(stake_id): Path<String>,
    Json(req): Json<WithdrawReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let row: Option<(String, String, bool)> =
        sqlx::query_as(r#"SELECT id, "userId", active FROM "Stake" WHERE id = $1"#)
            .bind(&stake_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(crate::api::internal)?;
    let Some((_, user_id, active)) = row else {
        return Err(not_found("Stake not found"));
    };
    if user_id != req.user_id {
        return Err(crate::api::forbidden("Not your stake"));
    }
    if !active {
        return Err(crate::api::conflict("Stake already withdrawn"));
    }

    let app_tag_id: String = sqlx::query_scalar(
        r#"UPDATE "Stake" SET active = false, "withdrawnAt" = now() WHERE id = $1 RETURNING "appTagId""#,
    )
    .bind(&stake_id)
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let app_id: String = sqlx::query_scalar(r#"SELECT "appId" FROM "AppTag" WHERE id = $1"#)
        .bind(&app_tag_id)
        .fetch_one(&state.pool)
        .await
        .map_err(crate::api::internal)?;

    refresh_app_tag(&state.pool, &app_tag_id).await?;
    refresh_app(&state.pool, &app_id).await?;

    Ok(Json(serde_json::json!({ "withdrawn": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawPartialReq {
    app_tag_id: String,
    user_id: String,
    amount: f64,
}

/// Withdraws `amount` (up to the full active total) off this (user, app-tag)'s
/// possibly-several active `Stake` rows — a user can stake on the same tag
/// more than once over time (each `stake_tag()` call just adds to the single
/// on-chain StakePosition), so this consumes rows oldest-first, fully
/// deactivating each until `amount` is covered and partially reducing the
/// last one it touches. Subsumes what a former `withdraw_all` did (the
/// special case of `amount` == the full total). Used by the profile page's
/// "Your stakes" list, which sums exactly these rows (see
/// handlers/rewards.rs) — the on-chain withdraw call there always withdraws
/// this same `amount`.
async fn withdraw_partial(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<WithdrawPartialReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.amount <= 0.0 {
        return Err(crate::api::bad_request("amount must be positive"));
    }

    let app_id: Option<String> = sqlx::query_scalar(r#"SELECT "appId" FROM "AppTag" WHERE id = $1"#)
        .bind(&req.app_tag_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(crate::api::internal)?;
    let Some(app_id) = app_id else {
        return Err(not_found("Tag not found"));
    };

    let rows: Vec<(String, f64)> = sqlx::query_as(
        r#"SELECT id, amount FROM "Stake" WHERE "userId" = $1 AND "appTagId" = $2 AND active = true ORDER BY "createdAt" ASC"#,
    )
    .bind(&req.user_id)
    .bind(&req.app_tag_id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    // See votes.rs's withdraw_partial for why this tolerance is safe.
    const EPS: f64 = 1e-9;
    let mut remaining = req.amount;
    for (id, row_amount) in rows {
        if remaining <= EPS {
            break;
        }
        if row_amount <= remaining + EPS {
            sqlx::query(r#"UPDATE "Stake" SET active = false, "withdrawnAt" = now() WHERE id = $1"#)
                .bind(&id)
                .execute(&state.pool)
                .await
                .map_err(crate::api::internal)?;
            remaining -= row_amount;
        } else {
            sqlx::query(r#"UPDATE "Stake" SET amount = amount - $2 WHERE id = $1"#)
                .bind(&id)
                .bind(remaining)
                .execute(&state.pool)
                .await
                .map_err(crate::api::internal)?;
            remaining = 0.0;
        }
    }

    if remaining > EPS {
        return Err(crate::api::bad_request("amount exceeds your active stake on this tag"));
    }

    refresh_app_tag(&state.pool, &req.app_tag_id).await?;
    refresh_app(&state.pool, &app_id).await?;

    Ok(Json(serde_json::json!({ "withdrawn": true })))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new()
        .route("/stakes", get(list).post(create))
        .route("/stakes/:id/withdraw", post(withdraw))
        .route("/stakes/withdraw-partial", post(withdraw_partial))
}
