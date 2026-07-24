//! Ports `app/src/app/api/rewards/positions/route.ts` — every app/tag the
//! given user has an active vote or stake on, collapsed to one aggregate
//! position per (app)/(app,tag) pair to match on-chain's single-position
//! model (see that file's doc comment for why the collapse is needed).

use crate::api::{ApiError, ApiState};
use axum::extract::{Json, Query, State};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionsQuery {
    user_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VotePositionDto {
    app_id: String,
    app_slug: String,
    app_name: String,
    app_icon_url: Option<String>,
    amount: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StakePositionDto {
    app_tag_id: String,
    app_id: String,
    app_slug: String,
    app_name: String,
    app_icon_url: Option<String>,
    tag_slug: String,
    tag_name: String,
    amount: f64,
}

async fn positions(State(state): State<Arc<ApiState>>, Query(q): Query<PositionsQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let vote_rows: Vec<(String, f64, String, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT v."appId", v.amount, a.slug, a.name, a."iconUrl"
        FROM "Vote" v JOIN "App" a ON a.id = v."appId"
        WHERE v."userId" = $1 AND v.active = true
        "#,
    )
    .bind(&q.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let mut vote_by_app: HashMap<String, VotePositionDto> = HashMap::new();
    for (app_id, amount, slug, name, icon_url) in vote_rows {
        vote_by_app
            .entry(app_id.clone())
            .and_modify(|v| v.amount += amount)
            .or_insert(VotePositionDto { app_id, app_slug: slug, app_name: name, app_icon_url: icon_url, amount });
    }

    type StakeRow = (String, f64, String, String, String, Option<String>, String, String);
    let stake_rows: Vec<StakeRow> = sqlx::query_as(
        r#"
        SELECT s."appTagId", s.amount, at."appId", a.slug, a.name, a."iconUrl", t.slug, t.name
        FROM "Stake" s
        JOIN "AppTag" at ON at.id = s."appTagId"
        JOIN "App" a ON a.id = at."appId"
        JOIN "Tag" t ON t.id = at."tagId"
        WHERE s."userId" = $1 AND s.active = true
        "#,
    )
    .bind(&q.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let mut stake_by_app_tag: HashMap<String, StakePositionDto> = HashMap::new();
    for (app_tag_id, amount, app_id, app_slug, app_name, app_icon_url, tag_slug, tag_name) in stake_rows {
        stake_by_app_tag
            .entry(app_tag_id.clone())
            .and_modify(|s| s.amount += amount)
            .or_insert(StakePositionDto {
                app_tag_id,
                app_id,
                app_slug,
                app_name,
                app_icon_url,
                tag_slug,
                tag_name,
                amount,
            });
    }

    Ok(Json(serde_json::json!({
        "votes": vote_by_app.into_values().collect::<Vec<_>>(),
        "stakes": stake_by_app_tag.into_values().collect::<Vec<_>>(),
    })))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new().route("/rewards/positions", get(positions))
}
