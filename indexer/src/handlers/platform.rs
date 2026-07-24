//! Ports `app/src/lib/appGraph.ts`, `app/src/lib/tagGraph.ts`, and
//! `app/src/lib/explore.ts` (platform-wide stats/trend for the Explore
//! page) — all read-only aggregate views over `App`/`AppTag`/`Tag`.

use crate::api::{ApiError, ApiState};
use axum::extract::{Json, Query, State};
use axum::routing::get;
use axum::Router;
use chrono::NaiveDateTime;
use crate::handlers::engine::to_rfc3339;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// --- appGraph.ts ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppGraphNode {
    id: String,
    name: String,
    stake: f64,
    views: i32,
    votes: f64,
}

#[derive(Serialize)]
struct AppGraphEdge {
    source: String,
    target: String,
    shared: f64,
    weighted: f64,
}

const MAX_NEIGHBORS_PER_APP: usize = 6;

/// How far back every platform-wide trend chart looks — apps/tags/votes/
/// stake (api.rs's get_platform_metrics_history), page views, and revenue
/// (both below) all share this window so they render on the same x-axis
/// domain instead of each defaulting to "however much history its own
/// source table happens to have". Before this, `platform_metrics_snapshot`
/// (a brand-new hourly gauge table) and `AppStatsSnapshot`/
/// `indexed_instruction` (long-lived daily tables) produced wildly
/// different date ranges on charts that are supposed to sit side by side.
/// Matches `TRENDING_MONTH_DAYS` in handlers/apps.rs — same "trending
/// month" concept, applied platform-wide instead of per-app.
pub const PLATFORM_TREND_WINDOW_DAYS: i64 = 30;

struct AppNode {
    slug: String,
    name: String,
    stake_total: f64,
    view_count: i32,
    vote_weight: f64,
    tags: Vec<(String, f64)>, // (tagId, stakeTotal)
}

fn edge_key(source: &str, target: &str) -> String {
    format!("{source}|{target}")
}

/// Same shape as `appGraph.ts`'s `buildAppGraph` — see that file's doc
/// comment for the Jaccard/weighted-Jaccard similarity rationale. `tags`
/// facet filtering (AND semantics) already happened at the call site, so
/// `apps` here is already the restricted set.
fn build_app_graph(apps: Vec<AppNode>) -> (Vec<AppGraphNode>, Vec<AppGraphEdge>) {
    let tagged: Vec<AppNode> = apps.into_iter().filter(|a| !a.tags.is_empty()).collect();

    let mut candidates: Vec<AppGraphEdge> = Vec::new();
    for i in 0..tagged.len() {
        for j in (i + 1)..tagged.len() {
            let a = &tagged[i];
            let b = &tagged[j];
            let b_tags: HashMap<&str, f64> = b.tags.iter().map(|(id, s)| (id.as_str(), *s)).collect();

            let mut intersection = 0i64;
            let mut sum_min = 0.0;
            for (tag_id, stake) in &a.tags {
                if let Some(&stake_b) = b_tags.get(tag_id.as_str()) {
                    intersection += 1;
                    sum_min += stake.min(stake_b);
                }
            }
            if intersection == 0 {
                continue;
            }
            let union = (a.tags.len() + b.tags.len()) as i64 - intersection;
            let shared = if union > 0 { intersection as f64 / union as f64 } else { 0.0 };
            let stake_union = sum_min.max(a.stake_total + b.stake_total - sum_min);
            let weighted = if stake_union > 0.0 { sum_min / stake_union } else { shared };

            candidates.push(AppGraphEdge { source: a.slug.clone(), target: b.slug.clone(), shared, weighted });
        }
    }

    let keep_shared = top_neighbor_keys(&candidates, |e| e.shared, MAX_NEIGHBORS_PER_APP);
    let keep_weighted = top_neighbor_keys(&candidates, |e| e.weighted, MAX_NEIGHBORS_PER_APP);
    let edges: Vec<AppGraphEdge> = candidates
        .into_iter()
        .filter(|e| {
            let key = edge_key(&e.source, &e.target);
            keep_shared.contains(&key) || keep_weighted.contains(&key)
        })
        .collect();

    let connected: HashSet<String> = edges.iter().flat_map(|e| [e.source.clone(), e.target.clone()]).collect();
    let nodes: Vec<AppGraphNode> = tagged
        .into_iter()
        .filter(|a| connected.contains(&a.slug))
        .map(|a| AppGraphNode { id: a.slug, name: a.name, stake: a.stake_total, views: a.view_count, votes: a.vote_weight })
        .collect();

    (nodes, edges)
}

fn top_neighbor_keys(edges: &[AppGraphEdge], weight_of: impl Fn(&AppGraphEdge) -> f64, k: usize) -> HashSet<String> {
    let mut by_node: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for e in edges {
        let key = edge_key(&e.source, &e.target);
        let weight = weight_of(e);
        for id in [&e.source, &e.target] {
            by_node.entry(id.clone()).or_default().push((key.clone(), weight));
        }
    }
    let mut keep = HashSet::new();
    for list in by_node.values_mut() {
        list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (key, _) in list.iter().take(k) {
            keep.insert(key.clone());
        }
    }
    keep
}

/// The maps' "advanced search" — min/max app stake, min/max tag count,
/// min/max pageviews. Same param names and semantics as `apps::SearchInput`
/// (which the Discover list's `FilterPanel` already exposes; see
/// `apps::in_range`, reused here), just applied over the graph/pack app
/// subsets instead of the full paginated search.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RangeQuery {
    #[serde(default)]
    app_stake_min: Option<f64>,
    #[serde(default)]
    app_stake_max: Option<f64>,
    #[serde(default)]
    tags_count_min: Option<i64>,
    #[serde(default)]
    tags_count_max: Option<i64>,
    #[serde(default)]
    pageviews_min: Option<f64>,
    #[serde(default)]
    pageviews_max: Option<f64>,
}

impl RangeQuery {
    fn matches(&self, stake: f64, view_count: i32, tag_count: i64) -> bool {
        crate::handlers::apps::in_range(stake, self.app_stake_min, self.app_stake_max)
            && crate::handlers::apps::in_range(view_count as f64, self.pageviews_min, self.pageviews_max)
            && crate::handlers::apps::in_range(
                tag_count as f64,
                self.tags_count_min.map(|v| v as f64),
                self.tags_count_max.map(|v| v as f64),
            )
    }
}

// `RangeQuery`'s fields are repeated here rather than `#[serde(flatten)]`ed
// in — axum's query-string extractor (serde_urlencoded under the hood)
// doesn't support flatten: every field query-deserializes fine standalone,
// but flattening it into a sibling struct makes deserialization fail with
// "invalid type: string ..., expected f64" for every request that includes
// even one range param (i.e. every "Advanced search" filter on the Apps
// map tab, unlike Group's tag_pack below, which takes `RangeQuery` directly
// and was never flattened, hence never demonstrated bug).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQuery {
    #[serde(default)]
    tags: Option<String>,
    #[serde(default)]
    app_stake_min: Option<f64>,
    #[serde(default)]
    app_stake_max: Option<f64>,
    #[serde(default)]
    tags_count_min: Option<i64>,
    #[serde(default)]
    tags_count_max: Option<i64>,
    #[serde(default)]
    pageviews_min: Option<f64>,
    #[serde(default)]
    pageviews_max: Option<f64>,
}

impl GraphQuery {
    fn ranges(&self) -> RangeQuery {
        RangeQuery {
            app_stake_min: self.app_stake_min,
            app_stake_max: self.app_stake_max,
            tags_count_min: self.tags_count_min,
            tags_count_max: self.tags_count_max,
            pageviews_min: self.pageviews_min,
            pageviews_max: self.pageviews_max,
        }
    }
}

async fn app_graph(State(state): State<Arc<ApiState>>, Query(q): Query<GraphQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let ranges = q.ranges();
    let tag_slugs: Vec<String> = q.tags.map(|s| s.split(',').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default();

    let rows: Vec<(String, String, f64, i32, f64)> = sqlx::query_as(
        r#"SELECT slug, name, "stakeTotal", "viewCount", "voteWeight" FROM "App" WHERE status = 'approved'"#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let mut apps = Vec::with_capacity(rows.len());
    for (slug, name, stake_total, view_count, vote_weight) in rows {
        let tag_rows: Vec<(String, f64, String)> = sqlx::query_as(
            r#"
            SELECT at."tagId", at."stakeTotal", t.slug
            FROM "AppTag" at JOIN "Tag" t ON t.id = at."tagId"
            JOIN "App" a ON a.id = at."appId"
            WHERE a.slug = $1
            "#,
        )
        .bind(&slug)
        .fetch_all(&state.pool)
        .await
        .map_err(crate::api::internal)?;

        // AND semantics: app must carry every selected tag slug.
        if !tag_slugs.is_empty() {
            let slugs: HashSet<&str> = tag_rows.iter().map(|(_, _, s)| s.as_str()).collect();
            if !tag_slugs.iter().all(|s| slugs.contains(s.as_str())) {
                continue;
            }
        }

        if !ranges.matches(stake_total, view_count, tag_rows.len() as i64) {
            continue;
        }

        apps.push(AppNode {
            slug,
            name,
            stake_total,
            view_count,
            vote_weight,
            tags: tag_rows.into_iter().map(|(id, stake, _)| (id, stake)).collect(),
        });
    }

    let (nodes, edges) = build_app_graph(apps);
    Ok(Json(serde_json::json!({ "nodes": nodes, "edges": edges })))
}

// --- tagGraph.ts ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TagGraphNode {
    id: String,
    name: String,
    stake: f64,
    app_count: i64,
}

#[derive(Serialize)]
struct TagGraphEdge {
    source: String,
    target: String,
    weight: i64,
    similarity: f64,
}

/// Same shape as `tagGraph.ts`'s `buildTagGraph`.
async fn tag_graph(State(state): State<Arc<ApiState>>) -> Result<Json<serde_json::Value>, ApiError> {
    let rows: Vec<(String, f64, String, String)> = sqlx::query_as(
        r#"
        SELECT at."appId", at."stakeTotal", t.slug, t.name
        FROM "AppTag" at
        JOIN "Tag" t ON t.id = at."tagId"
        JOIN "App" a ON a.id = at."appId"
        WHERE a.status = 'approved'
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let mut node_stake: HashMap<String, (String, f64, i64)> = HashMap::new(); // slug -> (name, stake, appCount)
    let mut by_app: HashMap<String, Vec<String>> = HashMap::new();
    for (app_id, stake, slug, name) in rows {
        let entry = node_stake.entry(slug.clone()).or_insert((name, 0.0, 0));
        entry.1 += stake;
        entry.2 += 1;
        by_app.entry(app_id).or_default().push(slug);
    }

    let mut edge_counts: HashMap<String, i64> = HashMap::new();
    for tags in by_app.values() {
        for i in 0..tags.len() {
            for j in (i + 1)..tags.len() {
                let mut pair = [tags[i].clone(), tags[j].clone()];
                pair.sort();
                *edge_counts.entry(format!("{}|{}", pair[0], pair[1])).or_insert(0) += 1;
            }
        }
    }

    let nodes: Vec<TagGraphNode> = node_stake
        .iter()
        .map(|(slug, (name, stake, app_count))| TagGraphNode { id: slug.clone(), name: name.clone(), stake: *stake, app_count: *app_count })
        .collect();

    let edges: Vec<TagGraphEdge> = edge_counts
        .into_iter()
        .map(|(key, weight)| {
            let (source, target) = key.split_once('|').unwrap();
            let count_a = node_stake.get(source).map(|(_, _, c)| *c).unwrap_or(0);
            let count_b = node_stake.get(target).map(|(_, _, c)| *c).unwrap_or(0);
            let union = count_a + count_b - weight;
            let similarity = if union > 0 { weight as f64 / union as f64 } else { 0.0 };
            TagGraphEdge { source: source.to_string(), target: target.to_string(), weight, similarity }
        })
        .collect();

    Ok(Json(serde_json::json!({ "nodes": nodes, "edges": edges })))
}

// --- tagPack.ts (Explore "Group" tab — circle-packing hierarchy) ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TagPackTagDto {
    slug: String,
    name: String,
    app_count: i64,
    stake: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TagPackAppDto {
    slug: String,
    name: String,
    stake: f64,
    tag_slugs: Vec<String>,
}

/// Raw material for a synthetic tag hierarchy: every approved app's full tag
/// list plus each tag's global popularity. There's no `parentId` on `Tag` —
/// the client derives "outer circle = most globally-common tag, inner
/// circles = next-most-common" by sorting each app's own tags into that
/// same global order (see app/src/lib/tagPack.ts's `buildTagPackTree`).
/// Same join as `tag_graph`, grouped by app instead of collapsed into edges.
async fn tag_pack(State(state): State<Arc<ApiState>>, Query(ranges): Query<RangeQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    // Which apps pass the range filters, decided up front — tag count needs
    // each app's full tag list, not available from the "App" table alone,
    // and deciding this first (rather than filtering the accumulated `apps`
    // map afterward) keeps `tag_stats`'s per-tag app-count/stake aggregates
    // consistent with the apps actually returned below.
    let app_rows: Vec<(String, f64, i32, i64)> = sqlx::query_as(
        r#"
        SELECT a.slug, a."stakeTotal", a."viewCount",
               (SELECT COUNT(*) FROM "AppTag" at WHERE at."appId" = a.id)
        FROM "App" a
        WHERE a.status = 'approved'
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;
    let passing: HashSet<String> = app_rows
        .into_iter()
        .filter(|(_, stake, views, tag_count)| ranges.matches(*stake, *views, *tag_count))
        .map(|(slug, ..)| slug)
        .collect();

    let rows: Vec<(String, String, f64, String, String, f64)> = sqlx::query_as(
        r#"
        SELECT a.slug, a.name, a."stakeTotal", t.slug, t.name, at."stakeTotal"
        FROM "AppTag" at
        JOIN "Tag" t ON t.id = at."tagId"
        JOIN "App" a ON a.id = at."appId"
        WHERE a.status = 'approved'
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let mut tag_stats: HashMap<String, (String, i64, f64)> = HashMap::new(); // tagSlug -> (name, appCount, stake)
    let mut apps: HashMap<String, (String, f64, Vec<String>)> = HashMap::new(); // appSlug -> (name, stake, tagSlugs)
    for (app_slug, app_name, app_stake, tag_slug, tag_name, tag_stake) in rows {
        if !passing.contains(&app_slug) {
            continue;
        }

        let tag_entry = tag_stats.entry(tag_slug.clone()).or_insert((tag_name, 0, 0.0));
        tag_entry.1 += 1;
        tag_entry.2 += tag_stake;

        let app_entry = apps.entry(app_slug).or_insert_with(|| (app_name, app_stake, Vec::new()));
        app_entry.2.push(tag_slug);
    }

    // Approved apps with zero tags never appear in the join above — fetched
    // separately so they still show up (as a synthetic "untagged" bucket in
    // the client's tree, rather than silently vanishing from the pack).
    let untagged_rows: Vec<(String, String, f64)> = sqlx::query_as(
        r#"
        SELECT a.slug, a.name, a."stakeTotal"
        FROM "App" a
        WHERE a.status = 'approved'
          AND NOT EXISTS (SELECT 1 FROM "AppTag" at WHERE at."appId" = a.id)
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;
    for (slug, name, stake) in untagged_rows {
        if !passing.contains(&slug) {
            continue;
        }
        apps.entry(slug).or_insert((name, stake, Vec::new()));
    }

    let tags: Vec<TagPackTagDto> = tag_stats
        .into_iter()
        .map(|(slug, (name, app_count, stake))| TagPackTagDto { slug, name, app_count, stake })
        .collect();
    let apps: Vec<TagPackAppDto> = apps
        .into_iter()
        .map(|(slug, (name, stake, tag_slugs))| TagPackAppDto { slug, name, stake, tag_slugs })
        .collect();

    Ok(Json(serde_json::json!({ "tags": tags, "apps": apps })))
}

// --- explore.ts ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformStatsDto {
    total_apps: i64,
    total_tags: i64,
    total_vote_weight: f64,
    total_stake: f64,
    total_views: i32,
    /// All-time sum of every `fund_app_rewards` instruction's `amount`, raw
    /// on-chain u64 (vote-token decimals) as a decimal string — see
    /// `RevenueTrendPointDto`'s comment below for why this reads from
    /// `indexed_instruction` rather than a dedicated table.
    total_revenue_distributed: String,
}

async fn platform_stats(State(state): State<Arc<ApiState>>) -> Result<Json<PlatformStatsDto>, ApiError> {
    let (total_apps, total_vote_weight, total_stake, total_views): (i64, Option<f64>, Option<f64>, Option<i64>) =
        sqlx::query_as(
            r#"SELECT COUNT(*), SUM("voteWeight"), SUM("stakeTotal"), SUM("viewCount") FROM "App" WHERE status = 'approved'"#,
        )
        .fetch_one(&state.pool)
        .await
        .map_err(crate::api::internal)?;

    let total_tags: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT at."tagId")
        FROM "AppTag" at JOIN "App" a ON a.id = at."appId"
        WHERE a.status = 'approved'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    let total_revenue_distributed: Option<String> = sqlx::query_scalar(
        r#"
        SELECT SUM((data->'data'->>'amount')::numeric)::text
        FROM indexed_instruction
        WHERE instruction_name = 'fund_app_rewards'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(Json(PlatformStatsDto {
        total_apps,
        total_tags,
        total_vote_weight: total_vote_weight.unwrap_or(0.0),
        total_stake: total_stake.unwrap_or(0.0),
        total_views: total_views.unwrap_or(0) as i32,
        total_revenue_distributed: total_revenue_distributed.unwrap_or_else(|| "0".to_string()),
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewsTrendPointDto {
    date: String,
    total_views: i64,
}

async fn platform_views_trend(State(state): State<Arc<ApiState>>) -> Result<Json<Vec<ViewsTrendPointDto>>, ApiError> {
    let rows: Vec<(NaiveDateTime, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT s.date, SUM(s."viewCount")
        FROM "AppStatsSnapshot" s JOIN "App" a ON a.id = s."appId"
        WHERE a.status = 'approved' AND s.date > now() - make_interval(days => $1)
        GROUP BY s.date
        ORDER BY s.date ASC
        "#,
    )
    .bind(PLATFORM_TREND_WINDOW_DAYS as i32)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|(date, total_views)| ViewsTrendPointDto { date: to_rfc3339(date), total_views: total_views.unwrap_or(0) })
            .collect(),
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevenueTrendPointDto {
    date: String,
    /// Raw on-chain u64 amount (vote-token decimals) summed for that day,
    /// as a decimal string — same convention as `PlatformMetricsPointDto`'s
    /// `total_vote_stake`/`total_tag_stake` in api.rs, for the same reason
    /// (a scaled u64 can exceed f64's safe integer range).
    amount: String,
}

/// Every `fund_app_rewards` call (the on-chain half of epoch settlement —
/// see `app/scripts/settleEpoch.ts`) is already captured, unconditionally
/// and idempotently, in `indexed_instruction` by the crawler's generic
/// instruction processor (`src/processors/instruction.rs`) — there's no
/// dedicated revenue table or Anchor event to listen for, since the
/// program never emits one. This aggregates that raw log directly rather
/// than adding a new periodic rollup, mirroring `platform_views_trend`
/// above. `data` is `{"type": "FundAppRewards", "data": {"pool": ...,
/// "amount": ...}}` per the decoder's `serde(tag = "type", content =
/// "data")` enum encoding, hence the nested `data->'data'->>'amount'`.
async fn platform_revenue_trend(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<Vec<RevenueTrendPointDto>>, ApiError> {
    let rows: Vec<(NaiveDateTime, Option<String>)> = sqlx::query_as(
        r#"
        SELECT date_trunc('day', block_time)::timestamp AS day,
               SUM((data->'data'->>'amount')::numeric)::text
        FROM indexed_instruction
        WHERE instruction_name = 'fund_app_rewards' AND block_time IS NOT NULL
          AND block_time > now() - make_interval(days => $1)
        GROUP BY day
        ORDER BY day ASC
        "#,
    )
    .bind(PLATFORM_TREND_WINDOW_DAYS as i32)
    .fetch_all(&state.pool)
    .await
    .map_err(crate::api::internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|(date, amount)| RevenueTrendPointDto {
                date: to_rfc3339(date),
                amount: amount.unwrap_or_else(|| "0".to_string()),
            })
            .collect(),
    ))
}

pub fn routes() -> Router<Arc<ApiState>> {
    Router::new()
        .route("/apps/graph", get(app_graph))
        .route("/tags/graph", get(tag_graph))
        .route("/tags/pack", get(tag_pack))
        .route("/platform/stats", get(platform_stats))
        .route("/platform/views-trend", get(platform_views_trend))
        .route("/platform/revenue-trend", get(platform_revenue_trend))
}
