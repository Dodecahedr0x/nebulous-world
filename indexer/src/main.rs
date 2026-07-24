mod api;
mod backfill;
mod config;
mod crawler;
mod db;
mod dlmm_bridge;
mod handlers;
mod opengraph;
mod platform_metrics;
mod processors;
mod reconcile;
mod rollup;

use anyhow::{Context, Result};
use carbon_core::pipeline::{Pipeline, ShutdownStrategy};
use carbon_nebulous_world_decoder::accounts::NebulousWorldAccount;
use carbon_nebulous_world_decoder::NebulousWorldDecoder;
use carbon_rpc_program_subscribe_datasource::{Filters as ProgramFilters, RpcProgramSubscribe};
use config::Config;
use processors::account::AccountProcessor;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use std::sync::Arc;

/// Indexes the nebulous_world Anchor program with Carbon
/// (https://github.com/sevenlabs-hq/carbon): the current state of every
/// account it owns (live via `programSubscribe`, backfilled at startup via
/// `getProgramAccounts`), every instruction sent to it (polled — see
/// src/crawler.rs for why `blockSubscribe` isn't used), and periodic
/// rollups of that data for visualization. Runs as a private Render
/// background service — no public HTTP surface, just this long-running
/// process plus the Postgres database it writes to (see render.yaml).
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let config = Config::from_env()?;
    log::info!(
        "starting indexer for program {} (rpc: {}, ws: {})",
        config.program_id,
        config.rpc_http_url,
        config.rpc_ws_url
    );

    let pool = db::connect(&config.database_url).await?;

    let backfill_result = backfill::run(&config.rpc_http_url, config.program_id, &pool).await?;
    reconcile::run(&pool, &backfill_result.decoded, config.vote_token_decimals).await?;

    // `Config.authority` is the admin pubkey `build_withdraw_vote`/
    // `build_withdraw_tag_stake` pay the unstake fee to (see
    // WithdrawVote::admin_token_account's doc comment in the program crate)
    // — read once here from the backfill's already-fetched account data
    // (Config never changes after `initialize`) rather than adding a
    // dedicated RPC round trip per withdrawal build. Missing entirely means
    // `initialize` hasn't run yet, which every other admin-gated flow in
    // this app already assumes has happened before the indexer is useful.
    let admin_authority = backfill_result
        .decoded
        .iter()
        .find_map(|(_, account)| match account {
            NebulousWorldAccount::Config(program_config) => Some(program_config.authority),
            _ => None,
        })
        .context("Config account not found — has `initialize` been called yet?")?;

    if let Err(e) = handlers::xp::backfill(&pool).await {
        log::warn!("xp backfill failed: {e}");
    }

    // Shared across the crawler's automatic OpenGraph enrichment
    // (src/opengraph.rs) and ApiState below — reqwest::Client pools
    // connections internally, so one instance for the whole process is the
    // idiomatic choice, not a new client per outbound request.
    let http_client = reqwest::Client::new();

    tokio::spawn(rollup::run(pool.clone(), config.rollup_interval_secs));
    tokio::spawn(crawler::run(
        config.rpc_http_url.clone(),
        config.program_id,
        pool.clone(),
        config.crawler_poll_interval_secs,
        http_client.clone(),
    ));
    tokio::spawn(platform_metrics::run(
        pool.clone(),
        config.platform_metrics_interval_secs,
    ));

    // The dlmm-bridge sidecar (see src/dlmm_bridge.rs) — spawned as a child
    // process rather than reimplemented in Rust; see dlmm-bridge/README.md.
    // Failing to spawn it is non-fatal: the rest of this API (account
    // reads, nebulous_world tx building/submission) works fine without it,
    // only /pool and /tx/buy-neb/build degrade.
    if let Err(e) = dlmm_bridge::spawn(&config).await {
        log::warn!("dlmm-bridge sidecar did not start (pool/buy-neb endpoints will fail): {e}");
    }

    let api_state = Arc::new(api::ApiState {
        pool: pool.clone(),
        rpc: solana_client::nonblocking::rpc_client::RpcClient::new(config.rpc_http_url.clone()),
        http: http_client,
        program_id: config.program_id,
        vote_token_mint: config.vote_token_mint.unwrap_or_default(),
        admin_authority,
        dlmm_bridge_url: config.dlmm_bridge_url.clone(),
    });
    let api_port = config.api_port;
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", api_port)).await {
            Ok(l) => l,
            Err(e) => {
                log::error!("failed to bind API server on port {api_port}: {e}");
                return;
            }
        };
        log::info!("HTTP API listening on 0.0.0.0:{api_port}");
        if let Err(e) = axum::serve(listener, api::router(api_state)).await {
            log::error!("API server exited: {e}");
        }
    });

    // Without an explicit encoding, programSubscribe defaults to one with a
    // small data-size limit that every account here exceeds (AppAccount's
    // variable-length app_id alone pushes it past that) — the WS stream
    // would silently fail to decode any real update. Base64 has no such
    // limit.
    let account_datasource = RpcProgramSubscribe::new(
        config.rpc_ws_url.clone(),
        ProgramFilters::new(
            config.program_id,
            Some(RpcProgramAccountsConfig {
                filters: None,
                account_config: RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    ..RpcAccountInfoConfig::default()
                },
                with_context: None,
                sort_results: None,
            }),
        ),
    );

    Pipeline::builder()
        .datasource(account_datasource)
        .account(NebulousWorldDecoder, AccountProcessor { pool })
        .shutdown_strategy(ShutdownStrategy::ProcessPending)
        .build()?
        .run()
        .await?;

    Ok(())
}
