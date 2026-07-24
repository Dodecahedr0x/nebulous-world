//! HTTP API the Next.js app talks to instead of a Solana RPC endpoint —
//! the whole point of this module: the app should never construct its own
//! `Connection`/RPC client, it should "rely entirely on the indexer" (see
//! the architecture note at the top of README.md). Every account read here
//! comes from `indexed_account` (already decoded by the Carbon pipeline —
//! no RPC call per request); every `/tx/*` endpoint builds an unsigned
//! transaction from that same indexed data plus a fresh blockhash (the one
//! RPC call this API does make, unavoidable since a transaction needs a
//! *current* blockhash); `/tx/submit` relays an already wallet-signed
//! transaction to the network. DLMM (Meteora) pool status/swap-building is
//! proxied to the `dlmm-bridge` sidecar (see dlmm-bridge/README.md for why
//! that one piece isn't native Rust).
//!
//! Internal-only (see render.yaml's `pserv` — no public HTTP surface):
//! the Next.js app's own API routes call this, the browser never does.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_transaction::Transaction;
use sqlx::types::Json as SqlxJson;
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::Arc;

use nebulous_world::constants::{
    APP_SEED, APP_TAG_STAKE_SEED, CONFIG_SEED, MAX_APP_ID_LEN, MAX_TAG_ID_LEN, STAKE_POSITION_SEED,
    TAG_SEED, VOTE_POSITION_SEED,
};

pub(crate) const TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const SYSTEM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("11111111111111111111111111111111111111");
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// Anchor instruction discriminators — copied verbatim from the matching
// `decode()` methods in decoder/src/instructions/*.rs (generated from the
// real on-chain IDL), so they're guaranteed correct without re-deriving
// sighash("global:<name>") by hand here.
const VOTE_DISC: [u8; 8] = [227, 110, 155, 23, 136, 126, 172, 25];
const WITHDRAW_VOTE_DISC: [u8; 8] = [243, 255, 70, 200, 3, 242, 103, 137];
const STAKE_TAG_DISC: [u8; 8] = [28, 227, 157, 227, 87, 132, 122, 89];
const WITHDRAW_TAG_STAKE_DISC: [u8; 8] = [56, 134, 3, 156, 20, 123, 219, 197];
const CLAIM_VOTE_REWARD_DISC: [u8; 8] = [113, 18, 86, 93, 183, 183, 117, 245];
const CLAIM_TAG_REWARD_DISC: [u8; 8] = [90, 104, 233, 219, 216, 183, 0, 2];
const INIT_APP_DISC: [u8; 8] = [126, 7, 32, 62, 17, 43, 172, 107];
const SUGGEST_TAG_DISC: [u8; 8] = [192, 92, 24, 181, 145, 125, 233, 31];
const CLOSE_VOTE_POSITION_DISC: [u8; 8] = [134, 150, 55, 9, 196, 152, 228, 22];
const CLOSE_TAG_STAKE_POSITION_DISC: [u8; 8] = [38, 121, 255, 37, 236, 94, 214, 33];

pub struct ApiState {
    pub pool: PgPool,
    pub rpc: RpcClient,
    pub http: reqwest::Client,
    pub program_id: Pubkey,
    pub vote_token_mint: Pubkey,
    /// `Config.authority` — the admin `build_withdraw_vote`/
    /// `build_withdraw_tag_stake` pay the unstake fee to directly (see
    /// `WithdrawVote::admin_token_account`'s doc comment in the program
    /// crate). Read once at startup in main.rs from the backfill's already-
    /// fetched `Config` account, not re-fetched per request.
    pub admin_authority: Pubkey,
    /// Base URL of the dlmm-bridge sidecar (e.g. http://127.0.0.1:8091) —
    /// see dlmm-bridge/README.md for why DLMM pool reads/swap-building are
    /// proxied there instead of implemented natively in Rust.
    pub dlmm_bridge_url: String,
}

pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/accounts/app/:app_id", get(get_app_account))
        .route(
            "/accounts/app-tag/:app_id/:tag_slug",
            get(get_app_tag_account),
        )
        .route(
            "/accounts/vote-position/:app_id/:owner",
            get(get_vote_position),
        )
        .route(
            "/accounts/stake-position/:app_id/:tag_slug/:owner",
            get(get_stake_position),
        )
        .route("/balances/:owner/:mint", get(get_balance))
        .route(
            "/wallet/:owner/closeable-positions",
            get(get_closeable_positions),
        )
        .route("/pool", get(get_pool))
        .route("/metrics/platform-history", get(get_platform_metrics_history))
        .route("/tx/create-app", post(build_create_app))
        .route("/tx/suggest-tag", post(build_suggest_tag))
        .route("/tx/vote", post(build_vote))
        .route("/tx/withdraw-vote", post(build_withdraw_vote))
        .route("/tx/stake-tag", post(build_stake_tag))
        .route("/tx/withdraw-tag-stake", post(build_withdraw_tag_stake))
        .route("/tx/claim-vote-reward", post(build_claim_vote_reward))
        .route("/tx/claim-tag-reward", post(build_claim_tag_reward))
        .route("/tx/claim-all-rewards", post(build_claim_all_rewards))
        .route("/tx/close-vote-position", post(build_close_vote_position))
        .route(
            "/tx/close-tag-stake-position",
            post(build_close_tag_stake_position),
        )
        .route("/tx/buy-neb/build", post(build_buy_neb))
        .route("/tx/submit", post(submit_tx))
        .merge(crate::handlers::users::routes())
        .merge(crate::handlers::apps::routes())
        .merge(crate::handlers::tags::routes())
        .merge(crate::handlers::platform::routes())
        .merge(crate::handlers::votes::routes())
        .merge(crate::handlers::stakes::routes())
        .merge(crate::handlers::rewards::routes())
        .merge(crate::handlers::ads::routes())
        .merge(crate::handlers::track::routes())
        .merge(crate::handlers::revenue::routes())
        .merge(crate::handlers::x402::routes())
        .merge(crate::handlers::xp::routes())
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

pub(crate) struct ApiError(pub(crate) StatusCode, pub(crate) String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

pub(crate) fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

pub(crate) fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, msg.into())
}

pub(crate) fn conflict(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::CONFLICT, msg.into())
}

pub(crate) fn forbidden(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::FORBIDDEN, msg.into())
}

pub(crate) fn internal(msg: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg.to_string())
}

fn parse_pubkey(field: &str, s: &str) -> Result<Pubkey, ApiError> {
    Pubkey::from_str(s).map_err(|_| bad_request(format!("invalid pubkey for {field}: {s}")))
}

fn parse_u64(field: &str, s: &str) -> Result<u64, ApiError> {
    s.parse::<u64>()
        .map_err(|_| bad_request(format!("invalid u64 for {field}: {s}")))
}

/// `app_id`/`tag_slug` become raw PDA seed bytes below (`app_pda`/`tag_pda`),
/// which Solana caps at 32 bytes each — see `MAX_APP_ID_LEN`/`MAX_TAG_ID_LEN`
/// on the program side. `Pubkey::find_program_address` doesn't return a
/// `Result` for an oversized seed, it PANICS ("Unable to find a viable
/// program address bump seed"), which takes down whichever request hit it
/// (confirmed live: a >32-byte app_id crashed the request's connection
/// outright rather than returning a 4xx). Reject oversized input before it
/// ever reaches a PDA derivation, the same way the on-chain program's own
/// `require!` does (see init_app.rs's/suggest_tag.rs's doc comments on why
/// that check is a backstop there but load-bearing here, since we don't
/// control call order the way the program's `try_accounts` does).
fn validate_seed_len(field: &str, s: &str, max: u8) -> Result<(), ApiError> {
    if s.len() > max as usize {
        return Err(bad_request(format!(
            "{field} must be at most {max} bytes, got {} bytes",
            s.len()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------
// PDA derivation — mirrors app/src/lib/anchorClient.ts exactly (same
// seeds, same order). Pure crypto, no RPC/DB involved.
// ---------------------------------------------------------------------

fn config_pda(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[CONFIG_SEED], program_id).0
}

fn app_pda(program_id: &Pubkey, app_id: &str) -> Pubkey {
    Pubkey::find_program_address(&[APP_SEED, app_id.as_bytes()], program_id).0
}

fn vote_position_pda(program_id: &Pubkey, app: &Pubkey, user: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[VOTE_POSITION_SEED, app.as_ref(), user.as_ref()],
        program_id,
    )
    .0
}

/// The GLOBAL tag identity — seeded only by the tag string, no `app`. The
/// same tag_id always derives the same `Tag` PDA no matter which app
/// suggested it.
fn tag_pda(program_id: &Pubkey, tag_id: &str) -> Pubkey {
    Pubkey::find_program_address(&[TAG_SEED, tag_id.as_bytes()], program_id).0
}

/// The stake-accounting connection for one (app, tag) pair — replaces the
/// old single `AppTagAccount` PDA now that tags are global (see `tag_pda`).
fn app_tag_stake_pda(program_id: &Pubkey, app: &Pubkey, tag: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[APP_TAG_STAKE_SEED, app.as_ref(), tag.as_ref()],
        program_id,
    )
    .0
}

fn stake_position_pda(program_id: &Pubkey, app_tag_stake: &Pubkey, user: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[STAKE_POSITION_SEED, app_tag_stake.as_ref(), user.as_ref()],
        program_id,
    )
    .0
}

pub(crate) fn associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

/// The single global vault every instruction transfers through — an ATA
/// owned by the `config` PDA (see programs/nebulous_world/src/state/config.rs).
fn vault_address(config: &Pubkey, mint: &Pubkey) -> Pubkey {
    associated_token_address(config, mint)
}

/// Builds an SPL Associated Token Account program `CreateIdempotent`
/// instruction (discriminator `1` — a no-op if the ATA already exists,
/// unlike discriminator `0`'s `Create`, which errors) — account layout
/// mirrors `@solana/spl-token`'s `createAssociatedTokenAccountIdempotentInstruction`
/// exactly: `[payer (signer, w), associated_token (w), owner, mint,
/// system_program, token_program]`.
///
/// Every builder below that reads/writes a user's vote-token ATA
/// (`user_token_account`) prepends this ahead of the real instruction,
/// unconditionally rather than behind an `account_exists` check — an
/// idempotent create against an already-existing ATA is a cheap no-op, so
/// there's nothing to gain from the extra RPC round-trip a check would
/// cost, and it closes the gap for any wallet that's never held the vote
/// token before (never bought NEB, so it has no ATA yet) or that closed an
/// emptied ATA between one call and the next (e.g. withdrew everything,
/// closed the account, then comes back to claim a reward later).
fn ata_create_idempotent_ix(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> Instruction {
    let associated_token = associated_token_address(owner, mint);
    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new(associated_token, false),
        AccountMeta::new_readonly(*owner, false),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];
    Instruction::new_with_bytes(ASSOCIATED_TOKEN_PROGRAM_ID, &[1u8], accounts)
}

/// Whether `pubkey` currently has an account on-chain — checked via a live
/// RPC call rather than the `indexed_account` cache (see `fetch_account`),
/// since the Carbon indexing pipeline can legitimately lag a slot or two
/// behind the tip. Used to decide whether a tx-building endpoint needs to
/// lazily prepend an `init_app`/`suggest_tag` instruction — a false "not
/// found" from a stale cache would incorrectly duplicate-create (or block)
/// an account that already exists on-chain.
async fn account_exists(rpc: &RpcClient, pubkey: &Pubkey) -> bool {
    rpc.get_account(pubkey).await.is_ok()
}

/// Borsh-encodes a `String` arg the way Anchor's IDL client does: a 4-byte
/// little-endian length prefix followed by the raw UTF-8 bytes. Shared by
/// `init_app_ix`/`suggest_tag_ix` below (each takes one or two String args).
fn push_borsh_string(data: &mut Vec<u8>, s: &str) {
    data.extend_from_slice(&(s.len() as u32).to_le_bytes());
    data.extend_from_slice(s.as_bytes());
}

/// Strips a leading `https://`/`http://` so the on-chain `AppAccount.url`
/// never pays rent for a prefix every app has anyway (see `MAX_URL_LEN`'s
/// doc comment) — the indexer's account processor prepends `https://` back
/// on when mirroring this into Postgres.
fn trim_url_protocol(url: &str) -> &str {
    url.trim_start_matches("https://").trim_start_matches("http://")
}

/// Builds a permissionless `init_app` instruction — see
/// programs/nebulous_world/src/instructions/init_app.rs's `InitApp` accounts
/// struct for the exact order/mutability this mirrors: `{ app, payer,
/// system_program }`.
fn init_app_ix(program_id: &Pubkey, app_id: &str, url: &str, app: &Pubkey, payer: &Pubkey) -> Instruction {
    let mut data = INIT_APP_DISC.to_vec();
    push_borsh_string(&mut data, app_id);
    push_borsh_string(&mut data, trim_url_protocol(url));
    let accounts = vec![
        AccountMeta::new(*app, false),
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];
    Instruction::new_with_bytes(*program_id, &data, accounts)
}

/// SPL Memo v2 program id — see `indexer/src/processors/product.rs`'s doc
/// comment for why a memo instruction carries the crowd-submitted app
/// metadata (name/tagline/description/...) that has no on-chain
/// `AppAccount` field of its own.
const MEMO_PROGRAM_ID: Pubkey = Pubkey::from_str_const("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

fn memo_ix(text: &str, signer: &Pubkey) -> Instruction {
    Instruction {
        program_id: MEMO_PROGRAM_ID,
        accounts: vec![AccountMeta::new_readonly(*signer, true)],
        data: text.as_bytes().to_vec(),
    }
}

/// Builds a permissionless `suggest_tag` instruction — mirrors
/// programs/nebulous_world/src/instructions/suggest_tag.rs's `SuggestTag`
/// accounts struct: `{ app, tag, app_tag_stake, payer, system_program }`.
/// `app` is read-only here (suggest_tag never mutates it, only reads
/// `app.bump` to validate its own seeds) — it must already exist on-chain
/// by the time this instruction executes, which callers guarantee by
/// prepending `init_app_ix` earlier in the same transaction when needed.
#[allow(clippy::too_many_arguments)]
fn suggest_tag_ix(
    program_id: &Pubkey,
    app_id: &str,
    tag_id: &str,
    app: &Pubkey,
    tag: &Pubkey,
    app_tag_stake: &Pubkey,
    payer: &Pubkey,
) -> Instruction {
    let mut data = SUGGEST_TAG_DISC.to_vec();
    push_borsh_string(&mut data, app_id);
    push_borsh_string(&mut data, tag_id);
    let accounts = vec![
        AccountMeta::new_readonly(*app, false),
        AccountMeta::new(*tag, false),
        AccountMeta::new(*app_tag_stake, false),
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];
    Instruction::new_with_bytes(*program_id, &data, accounts)
}

// ---------------------------------------------------------------------
// Account reads — straight from `indexed_account`'s JSONB, deserialized
// directly into a concrete struct (never into a generic serde_json::Value)
// so u64/u128 fields keep full precision — see the module doc on
// AppAccountRow for why this matters.
// ---------------------------------------------------------------------

/// Mirrors decoder/src/accounts/app_account.rs field-for-field (same
/// names, since that struct has no #[serde(rename)] — its JSON keys are
/// its Rust field names verbatim). Deserializing straight into typed u64/
/// u128 fields (rather than through a generic serde_json::Value, whose
/// Number type can't hold u128 and silently loses precision above 2^64)
/// is what keeps `vote_acc_reward_per_share`/`tags_acc_reward_per_share`
/// exact.
#[derive(Deserialize)]
struct AppAccountRow {
    app_id: String,
    total_vote_stake: u64,
    vote_acc_reward_per_share: u128,
    total_tag_stake: u64,
    tags_acc_reward_per_share: u128,
    bump: u8,
}

#[derive(Deserialize)]
struct AppTagStakeRow {
    app: Pubkey,
    tag: Pubkey,
    stake_amount: u64,
    bump: u8,
}

#[derive(Deserialize)]
struct PositionRow {
    owner: Pubkey,
    /// Who paid this position's rent at creation — see
    /// `VotePosition::payer`/`StakePosition::payer`'s doc comments in the
    /// program crate. `build_close_vote_position`/`build_close_tag_stake_position`
    /// read this to fill in the close instruction's `payer` account, since
    /// it isn't a PDA seed and can't be re-derived, only read.
    payer: Pubkey,
    amount: u64,
    reward_debt: u128,
    /// Unix seconds — see `VotePosition::staked_at`/`StakePosition::staked_at`'s
    /// doc comments in the program crate. Drives the client-side unstake-fee
    /// estimate (`app/src/lib/unstakeFee.ts` mirrors the same decay constants).
    staked_at: i64,
    bump: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppAccountDto {
    pda: String,
    app_id: String,
    total_vote_stake: String,
    vote_acc_reward_per_share: String,
    total_tag_stake: String,
    tags_acc_reward_per_share: String,
    bump: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppTagStakeDto {
    pda: String,
    app: String,
    tag: String,
    /// Not read off the account (it no longer stores this — the tag_id
    /// string lives on the separate global `Tag` account instead): this is
    /// simply the `tag_slug` path param, which is exactly the string that
    /// derived `tag` above.
    tag_id: String,
    stake_amount: String,
    bump: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PositionDto {
    pda: String,
    owner: String,
    payer: String,
    amount: String,
    reward_debt: String,
    /// Unix seconds (not milliseconds, not an ISO string) — the raw on-chain
    /// `staked_at` checkpoint. Small enough to be a safe JS number (current
    /// Unix time is ~10 digits, nowhere near `Number.MAX_SAFE_INTEGER`), so
    /// unlike the u64/u128 fields above this isn't a decimal string.
    staked_at: i64,
    bump: u8,
}

/// Fetches one row's `data` JSONB column, deserialized into `T`, or `Ok(None)`
/// if no row exists for that pubkey/account_type pair — a 404 to callers,
/// not an error, since e.g. a VotePosition legitimately doesn't exist until
/// a user's first vote.
async fn fetch_account<T: serde::de::DeserializeOwned + Send + Unpin + 'static>(
    pool: &PgPool,
    pubkey: &Pubkey,
    account_type: &str,
) -> Result<Option<T>, ApiError> {
    let row: Option<(SqlxJson<T>,)> = sqlx::query_as(
        // `data` is `{"type": "<AccountType>", "data": {...fields}}` — the
        // Codama-generated `NebulousWorldAccount` enum's serde-tagged
        // representation (see decoder/src/accounts/mod.rs). `->'data'`
        // unwraps straight to the fields, which the account_type column
        // (checked below) already disambiguates without needing the tag.
        "SELECT data->'data' FROM indexed_account WHERE pubkey = $1 AND account_type = $2",
    )
    .bind(pubkey.to_string())
    .bind(account_type)
    .fetch_optional(pool)
    .await
    .map_err(internal)?;
    Ok(row.map(|(SqlxJson(v),)| v))
}

async fn get_app_account(
    State(state): State<Arc<ApiState>>,
    Path(app_id): Path<String>,
) -> Result<Json<AppAccountDto>, ApiError> {
    validate_seed_len("app_id", &app_id, MAX_APP_ID_LEN)?;
    let pda = app_pda(&state.program_id, &app_id);
    let row: AppAccountRow = fetch_account(&state.pool, &pda, "AppAccount")
        .await?
        .ok_or_else(|| not_found(format!("app {app_id} is not indexed yet")))?;
    Ok(Json(AppAccountDto {
        pda: pda.to_string(),
        app_id: row.app_id,
        total_vote_stake: row.total_vote_stake.to_string(),
        vote_acc_reward_per_share: row.vote_acc_reward_per_share.to_string(),
        total_tag_stake: row.total_tag_stake.to_string(),
        tags_acc_reward_per_share: row.tags_acc_reward_per_share.to_string(),
        bump: row.bump,
    }))
}

async fn get_app_tag_account(
    State(state): State<Arc<ApiState>>,
    Path((app_id, tag_slug)): Path<(String, String)>,
) -> Result<Json<AppTagStakeDto>, ApiError> {
    validate_seed_len("app_id", &app_id, MAX_APP_ID_LEN)?;
    validate_seed_len("tag_slug", &tag_slug, MAX_TAG_ID_LEN)?;
    let app = app_pda(&state.program_id, &app_id);
    let tag = tag_pda(&state.program_id, &tag_slug);
    let pda = app_tag_stake_pda(&state.program_id, &app, &tag);
    let row: AppTagStakeRow = fetch_account(&state.pool, &pda, "AppTagStake")
        .await?
        .ok_or_else(|| not_found(format!("tag {tag_slug} on app {app_id} is not indexed yet")))?;
    Ok(Json(AppTagStakeDto {
        pda: pda.to_string(),
        app: row.app.to_string(),
        tag: row.tag.to_string(),
        tag_id: tag_slug,
        stake_amount: row.stake_amount.to_string(),
        bump: row.bump,
    }))
}

async fn get_vote_position(
    State(state): State<Arc<ApiState>>,
    Path((app_id, owner)): Path<(String, String)>,
) -> Result<Json<PositionDto>, ApiError> {
    validate_seed_len("app_id", &app_id, MAX_APP_ID_LEN)?;
    let owner_pk = parse_pubkey("owner", &owner)?;
    let app = app_pda(&state.program_id, &app_id);
    let pda = vote_position_pda(&state.program_id, &app, &owner_pk);
    let row: PositionRow = fetch_account(&state.pool, &pda, "VotePosition")
        .await?
        .ok_or_else(|| not_found("no vote position yet".to_string()))?;
    Ok(Json(PositionDto {
        pda: pda.to_string(),
        owner: row.owner.to_string(),
        payer: row.payer.to_string(),
        amount: row.amount.to_string(),
        reward_debt: row.reward_debt.to_string(),
        staked_at: row.staked_at,
        bump: row.bump,
    }))
}

async fn get_stake_position(
    State(state): State<Arc<ApiState>>,
    Path((app_id, tag_slug, owner)): Path<(String, String, String)>,
) -> Result<Json<PositionDto>, ApiError> {
    validate_seed_len("app_id", &app_id, MAX_APP_ID_LEN)?;
    validate_seed_len("tag_slug", &tag_slug, MAX_TAG_ID_LEN)?;
    let owner_pk = parse_pubkey("owner", &owner)?;
    let app = app_pda(&state.program_id, &app_id);
    let tag = tag_pda(&state.program_id, &tag_slug);
    let app_tag_stake = app_tag_stake_pda(&state.program_id, &app, &tag);
    let pda = stake_position_pda(&state.program_id, &app_tag_stake, &owner_pk);
    let row: PositionRow = fetch_account(&state.pool, &pda, "StakePosition")
        .await?
        .ok_or_else(|| not_found("no stake position yet".to_string()))?;
    Ok(Json(PositionDto {
        pda: pda.to_string(),
        owner: row.owner.to_string(),
        payer: row.payer.to_string(),
        amount: row.amount.to_string(),
        reward_debt: row.reward_debt.to_string(),
        staked_at: row.staked_at,
        bump: row.bump,
    }))
}

// ---------------------------------------------------------------------
// Wallet token balances — a single targeted `getTokenAccountBalance` per
// request, not a `getProgramAccounts` scan, so this doesn't fall under the
// "getProgramAccounts only at startup" constraint (that's about the one
// RPC method that scans every account owned by a program — a per-owner
// balance lookup is a normal, cheap, targeted read). Runs indexer-side so
// the app itself never touches RPC.
// ---------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BalanceDto {
    amount: String,
    decimals: u8,
    ui_amount_string: String,
}

async fn get_balance(
    State(state): State<Arc<ApiState>>,
    Path((owner, mint)): Path<(String, String)>,
) -> Result<Json<BalanceDto>, ApiError> {
    let owner_pk = parse_pubkey("owner", &owner)?;
    let mint_pk = parse_pubkey("mint", &mint)?;
    let ata = associated_token_address(&owner_pk, &mint_pk);
    match state.rpc.get_token_account_balance(&ata).await {
        Ok(balance) => Ok(Json(BalanceDto {
            amount: balance.amount,
            decimals: balance.decimals,
            ui_amount_string: balance.ui_amount_string,
        })),
        // No ATA yet (wallet never held this token) — a zero balance, not
        // an error, mirroring useWalletBalances.ts's prior client-side
        // try/catch-to-zero behavior.
        Err(_) => Ok(Json(BalanceDto {
            amount: "0".to_string(),
            decimals: 0,
            ui_amount_string: "0".to_string(),
        })),
    }
}

// ---------------------------------------------------------------------
// DLMM pool status / swap building — proxied to the sidecar.
// ---------------------------------------------------------------------

async fn get_pool(State(state): State<Arc<ApiState>>) -> Result<Response, ApiError> {
    proxy_get(&state, "/pool").await
}

async fn build_buy_neb(
    State(state): State<Arc<ApiState>>,
    body: Json<serde_json::Value>,
) -> Result<Response, ApiError> {
    proxy_post(&state, "/tx/buy-neb/build", body.0).await
}

async fn proxy_get(state: &Arc<ApiState>, path: &str) -> Result<Response, ApiError> {
    let url = format!("{}{}", state.dlmm_bridge_url, path);
    let resp = state.http.get(&url).send().await.map_err(internal)?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = resp.text().await.map_err(internal)?;
    Ok((status, [("content-type", "application/json")], body).into_response())
}

async fn proxy_post(
    state: &Arc<ApiState>,
    path: &str,
    body: serde_json::Value,
) -> Result<Response, ApiError> {
    let url = format!("{}{}", state.dlmm_bridge_url, path);
    let resp = state
        .http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(internal)?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let text = resp.text().await.map_err(internal)?;
    Ok((status, [("content-type", "application/json")], text).into_response())
}

// ---------------------------------------------------------------------
// Transaction building — raw instruction construction (discriminator +
// borsh-encoded args, both copied from the generated decoder) rather than
// pulling in anchor-client. `nebulous_world` is still a Cargo path
// dependency (see Cargo.toml) — but only for its `constants` module (PDA
// seed bytes), never as an Anchor CPI/client crate. Account order and
// is_signer/is_writable flags mirror programs/nebulous_world/src/
// instructions/*.rs's `#[derive(Accounts)]` structs exactly.
// ---------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BuiltTxDto {
    transaction: String,
}

/// Takes a `Vec` (rather than a single `Instruction`) because several
/// callers below need to lazily prepend an `init_app`/`suggest_tag`
/// instruction ahead of the "real" one, all landing in one atomic,
/// one-signature transaction.
async fn unsigned_tx_base64(
    rpc: &RpcClient,
    fee_payer: &Pubkey,
    instructions: Vec<Instruction>,
) -> Result<String, ApiError> {
    let blockhash: Hash = rpc.get_latest_blockhash().await.map_err(internal)?;
    let message = Message::new_with_blockhash(&instructions, Some(fee_payer), &blockhash);
    let tx = Transaction::new_unsigned(message);
    let bytes = bincode::serialize(&tx).map_err(internal)?;
    Ok(BASE64.encode(bytes))
}

/// Metadata with no on-chain `AppAccount` field of its own (see
/// `programs/nebulous_world/src/state/app.rs`'s doc comment) — attached to
/// the creation transaction as an SPL Memo instruction, whose JSON shape
/// exactly matches `indexer/src/processors/product.rs`'s private `AppMemo`
/// (the two are independently defined, one per side of the wire, rather
/// than shared, since one is this crate's outbound DTO and the other is
/// that module's inbound parse target — keep them in sync by hand).
#[derive(Serialize, Default)]
#[serde(rename_all = "snake_case")]
struct CreateAppMemo {
    name: Option<String>,
    tagline: Option<String>,
    description: Option<String>,
    icon_url: Option<String>,
    category: Option<String>,
    chain: Option<String>,
}

impl CreateAppMemo {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.tagline.is_none()
            && self.description.is_none()
            && self.icon_url.is_none()
            && self.category.is_none()
            && self.chain.is_none()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAppReq {
    app_id: String,
    url: String,
    user: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tagline: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    chain: Option<String>,
}

/// Builds "create app (+ optional initial tags)" as ONE atomic,
/// one-signature transaction — the on-chain-first replacement for the old
/// `POST /api/apps` Prisma write (see AGENTS.md and processors/product.rs):
/// `init_app` (with an optional leading Memo carrying metadata that has no
/// on-chain field), then one `suggest_tag` per initial tag. The `App`/`Tag`/
/// `AppTag` Postgres rows don't exist yet when this returns — they show up
/// once the indexer's crawler observes the confirmed transaction, same as
/// every other on-chain action in this app.
async fn build_create_app(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<CreateAppReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    if req.url.trim().is_empty() {
        return Err(bad_request("url must not be empty"));
    }
    if req.tags.len() > 10 {
        return Err(bad_request("at most 10 initial tags are allowed"));
    }
    for tag_slug in &req.tags {
        validate_seed_len("tag_slug", tag_slug, MAX_TAG_ID_LEN)?;
    }
    let user = parse_pubkey("user", &req.user)?;
    let app = app_pda(&state.program_id, &req.app_id);

    if account_exists(&state.rpc, &app).await {
        return Err(bad_request(format!(
            "an app with id \"{}\" already exists on-chain",
            req.app_id
        )));
    }

    let mut instructions = Vec::new();

    let memo = CreateAppMemo {
        name: req.name.clone(),
        tagline: req.tagline.clone(),
        description: req.description.clone(),
        icon_url: req.icon_url.clone(),
        category: req.category.clone(),
        chain: req.chain.clone(),
    };
    if !memo.is_empty() {
        let memo_json = serde_json::to_string(&memo).map_err(internal)?;
        instructions.push(memo_ix(&memo_json, &user));
    }

    instructions.push(init_app_ix(&state.program_id, &req.app_id, &req.url, &app, &user));

    for tag_slug in &req.tags {
        let tag = tag_pda(&state.program_id, tag_slug);
        let app_tag_stake = app_tag_stake_pda(&state.program_id, &app, &tag);
        instructions.push(suggest_tag_ix(
            &state.program_id,
            &req.app_id,
            tag_slug,
            &app,
            &tag,
            &app_tag_stake,
            &user,
        ));
    }

    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuggestTagReq {
    app_id: String,
    tag_slug: String,
    user: String,
}

/// Adds a tag to an app that already exists (unlike `build_create_app`,
/// which bundles this alongside `init_app` for an app's INITIAL tags) — the
/// on-chain-first replacement for the old `POST /api/tags/suggest` Prisma
/// write. `app` must already exist; `suggest_tag` only reads it, so there's
/// no lazy `init_app` fallback here (same reasoning as `build_vote`).
async fn build_suggest_tag(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SuggestTagReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    validate_seed_len("tag_slug", &req.tag_slug, MAX_TAG_ID_LEN)?;
    let user = parse_pubkey("user", &req.user)?;
    let app = app_pda(&state.program_id, &req.app_id);
    let tag = tag_pda(&state.program_id, &req.tag_slug);
    let app_tag_stake = app_tag_stake_pda(&state.program_id, &app, &tag);

    let instructions = vec![suggest_tag_ix(
        &state.program_id,
        &req.app_id,
        &req.tag_slug,
        &app,
        &tag,
        &app_tag_stake,
        &user,
    )];
    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoteReq {
    app_id: String,
    amount: String,
    user: String,
}

async fn build_vote(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<VoteReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    let user = parse_pubkey("user", &req.user)?;
    let amount = parse_u64("amount", &req.amount)?;
    let app = app_pda(&state.program_id, &req.app_id);
    let position = vote_position_pda(&state.program_id, &app, &user);
    let config = config_pda(&state.program_id);
    let vault = vault_address(&config, &state.vote_token_mint);
    let user_token_account = associated_token_address(&user, &state.vote_token_mint);

    // `vote` requires `AppAccount` to already exist. Unlike before, there's
    // no lazy `init_app` prepend here any more: app creation is now an
    // on-chain-first flow of its own (POST /tx/create-app, built by
    // `build_create_app`) that always runs before any app can be voted on,
    // and (unlike the old Postgres-first design) this endpoint has no `url`
    // to construct `init_app` with even if it wanted to lazily create one.
    // A vote against a nonexistent `app` now simply fails on-chain with the
    // ordinary "account not found" error, which is the correct behavior.
    //
    // `user_token_account` isn't guaranteed to exist yet (a wallet's first
    // ever vote may be its first ever contact with the vote token) — see
    // `ata_create_idempotent_ix`'s doc comment.
    let mut instructions = vec![ata_create_idempotent_ix(&user, &user, &state.vote_token_mint)];

    let mut data = VOTE_DISC.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(app, false),
        AccountMeta::new(position, false),
        AccountMeta::new_readonly(config, false),
        AccountMeta::new(vault, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new(user, true),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];
    instructions.push(Instruction::new_with_bytes(state.program_id, &data, accounts));
    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

async fn build_withdraw_vote(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<VoteReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    let user = parse_pubkey("user", &req.user)?;
    let amount = parse_u64("amount", &req.amount)?;
    let app = app_pda(&state.program_id, &req.app_id);
    let position = vote_position_pda(&state.program_id, &app, &user);
    // Unlike `vote`, a withdrawal can only ever target a position on an app
    // that genuinely already exists — there is nothing to lazily create
    // here, so a missing account is a real error, not an auto-init case.
    if !account_exists(&state.rpc, &app).await {
        return Err(not_found(format!("app {} does not exist on-chain yet", req.app_id)));
    }
    let config = config_pda(&state.program_id);
    let vault = vault_address(&config, &state.vote_token_mint);
    let user_token_account = associated_token_address(&user, &state.vote_token_mint);
    let admin_token_account = associated_token_address(&state.admin_authority, &state.vote_token_mint);

    let mut data = WITHDRAW_VOTE_DISC.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());

    // No system_program — WithdrawVote never inits an account (unlike Vote's
    // init_if_needed position). `user` is a signer but not writable — this
    // instruction never debits/credits the fee payer's own lamports balance
    // beyond the network fee (which doesn't require is_writable on the
    // signer field itself).
    let accounts = vec![
        AccountMeta::new(app, false),
        AccountMeta::new(position, false),
        AccountMeta::new_readonly(config, false),
        AccountMeta::new(vault, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new(admin_token_account, false),
        AccountMeta::new_readonly(user, true),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];
    // `user_token_account` normally still exists from the vote this
    // withdraws (voting already requires it — see `build_vote`), but a
    // wallet could have closed it in between while it sat empty (all its
    // balance moved into `vault` at vote time) — see
    // `ata_create_idempotent_ix`'s doc comment. `admin_token_account` has no
    // such prior-vote guarantee — the admin may never have received a fee
    // before — so it needs the same idempotent-create treatment, just paid
    // for by `user` (the withdrawer) rather than the admin themselves, same
    // as every other caller-created ATA this API idempotently provisions.
    let instructions = vec![
        ata_create_idempotent_ix(&user, &user, &state.vote_token_mint),
        ata_create_idempotent_ix(&user, &state.admin_authority, &state.vote_token_mint),
        Instruction::new_with_bytes(state.program_id, &data, accounts),
    ];
    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StakeTagReq {
    app_id: String,
    tag_slug: String,
    amount: String,
    user: String,
}

async fn build_stake_tag(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<StakeTagReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    validate_seed_len("tag_slug", &req.tag_slug, MAX_TAG_ID_LEN)?;
    let user = parse_pubkey("user", &req.user)?;
    let amount = parse_u64("amount", &req.amount)?;
    let app = app_pda(&state.program_id, &req.app_id);
    let tag = tag_pda(&state.program_id, &req.tag_slug);
    let app_tag_stake = app_tag_stake_pda(&state.program_id, &app, &tag);
    let position = stake_position_pda(&state.program_id, &app_tag_stake, &user);
    let config = config_pda(&state.program_id);
    let vault = vault_address(&config, &state.vote_token_mint);
    let user_token_account = associated_token_address(&user, &state.vote_token_mint);

    // `app` must already exist (see the comment on `build_vote` — app
    // creation is its own on-chain-first flow now, `POST /tx/create-app`,
    // with no lazy `init_app` fallback here any more). `tag`/`app_tag_stake`
    // are different: `suggest_tag` is permissionless and legitimately gets
    // called again later, against an already-existing app, for tags that
    // weren't part of the original creation transaction — lazily create
    // whichever of those two is still missing ahead of the real
    // `stake_tag` instruction.
    let mut instructions = Vec::new();
    if !account_exists(&state.rpc, &app_tag_stake).await {
        instructions.push(suggest_tag_ix(
            &state.program_id,
            &req.app_id,
            &req.tag_slug,
            &app,
            &tag,
            &app_tag_stake,
            &user,
        ));
    }
    // `user_token_account` isn't guaranteed to exist yet, same reasoning as
    // `build_vote` — see `ata_create_idempotent_ix`'s doc comment.
    instructions.push(ata_create_idempotent_ix(&user, &user, &state.vote_token_mint));

    let mut data = STAKE_TAG_DISC.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(app, false),
        AccountMeta::new(app_tag_stake, false),
        AccountMeta::new(position, false),
        AccountMeta::new_readonly(config, false),
        AccountMeta::new(vault, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new(user, true),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];
    instructions.push(Instruction::new_with_bytes(state.program_id, &data, accounts));
    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

async fn build_withdraw_tag_stake(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<StakeTagReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    validate_seed_len("tag_slug", &req.tag_slug, MAX_TAG_ID_LEN)?;
    let user = parse_pubkey("user", &req.user)?;
    let amount = parse_u64("amount", &req.amount)?;
    let app = app_pda(&state.program_id, &req.app_id);
    let tag = tag_pda(&state.program_id, &req.tag_slug);
    let app_tag_stake = app_tag_stake_pda(&state.program_id, &app, &tag);
    let position = stake_position_pda(&state.program_id, &app_tag_stake, &user);
    // As in `withdraw_vote`: a withdrawal can only ever target stake that
    // genuinely already exists on-chain — nothing to lazily create here.
    if !account_exists(&state.rpc, &app).await {
        return Err(not_found(format!("app {} does not exist on-chain yet", req.app_id)));
    }
    if !account_exists(&state.rpc, &app_tag_stake).await {
        return Err(not_found(format!("tag {} does not exist on-chain yet", req.tag_slug)));
    }
    let config = config_pda(&state.program_id);
    let vault = vault_address(&config, &state.vote_token_mint);
    let user_token_account = associated_token_address(&user, &state.vote_token_mint);
    let admin_token_account = associated_token_address(&state.admin_authority, &state.vote_token_mint);

    let mut data = WITHDRAW_TAG_STAKE_DISC.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(app, false),
        AccountMeta::new(app_tag_stake, false),
        AccountMeta::new(position, false),
        AccountMeta::new_readonly(config, false),
        AccountMeta::new(vault, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new(admin_token_account, false),
        AccountMeta::new_readonly(user, true),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];
    // See `build_withdraw_vote`'s comment on the same pattern.
    let instructions = vec![
        ata_create_idempotent_ix(&user, &user, &state.vote_token_mint),
        ata_create_idempotent_ix(&user, &state.admin_authority, &state.vote_token_mint),
        Instruction::new_with_bytes(state.program_id, &data, accounts),
    ];
    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

// ---------------------------------------------------------------------
// Closing emptied positions — reclaims a `VotePosition`/`StakePosition`'s
// rent once its `amount` is back to 0, refunded to whoever originally paid
// it (`PositionRow::payer`, read off the indexed account itself: it isn't a
// PDA seed, so it can only be looked up, not re-derived). Both endpoints
// take the position's own pubkey directly rather than app_id (+tag_slug) —
// the on-chain instructions re-derive their seeds from the position
// account's own stored `app`/`app_tag_stake` field (see
// `close_vote_position.rs`'s doc comment), so nothing else is needed to
// build a valid close instruction, and the caller already has the pubkey
// from `GET /wallet/:owner/closeable-positions` below.
// ---------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClosePositionReq {
    position: String,
    user: String,
}

async fn build_close_vote_position(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ClosePositionReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    let user = parse_pubkey("user", &req.user)?;
    let position = parse_pubkey("position", &req.position)?;
    let row: PositionRow = fetch_account(&state.pool, &position, "VotePosition")
        .await?
        .ok_or_else(|| not_found("vote position does not exist".to_string()))?;
    if row.owner != user {
        return Err(forbidden("user is not this position's owner".to_string()));
    }
    if row.amount != 0 {
        return Err(conflict("position still holds stake — withdraw it first".to_string()));
    }

    let data = CLOSE_VOTE_POSITION_DISC.to_vec();
    // `payer` is writable (it receives the reclaimed rent lamports) but
    // never needs to sign — see close_vote_position.rs's doc comment on why
    // receiving lamports back requires no authorization from the receiver.
    let accounts = vec![
        AccountMeta::new(position, false),
        AccountMeta::new(row.payer, false),
        AccountMeta::new_readonly(user, true),
    ];
    let ix = Instruction::new_with_bytes(state.program_id, &data, accounts);
    let tx = unsigned_tx_base64(&state.rpc, &user, vec![ix]).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

async fn build_close_tag_stake_position(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ClosePositionReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    let user = parse_pubkey("user", &req.user)?;
    let position = parse_pubkey("position", &req.position)?;
    let row: PositionRow = fetch_account(&state.pool, &position, "StakePosition")
        .await?
        .ok_or_else(|| not_found("tag stake position does not exist".to_string()))?;
    if row.owner != user {
        return Err(forbidden("user is not this position's owner".to_string()));
    }
    if row.amount != 0 {
        return Err(conflict("position still holds stake — withdraw it first".to_string()));
    }

    let data = CLOSE_TAG_STAKE_POSITION_DISC.to_vec();
    let accounts = vec![
        AccountMeta::new(position, false),
        AccountMeta::new(row.payer, false),
        AccountMeta::new_readonly(user, true),
    ];
    let ix = Instruction::new_with_bytes(state.program_id, &data, accounts);
    let tx = unsigned_tx_base64(&state.rpc, &user, vec![ix]).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloseablePositionDto {
    position: String,
    kind: &'static str,
    /// Rent lamports this position will refund to its payer once closed —
    /// small enough (a rent-exempt minimum, not a token amount) to safely
    /// be a plain JS number rather than a decimal string.
    lamports: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloseablePositionsDto {
    positions: Vec<CloseablePositionDto>,
}

/// Every `VotePosition`/`StakePosition` owned by `owner` that currently
/// holds zero stake — exactly the set `build_close_vote_position`/
/// `build_close_tag_stake_position` above are safe to call against, and
/// what the "close all 0-stake accounts" UI action lists. A direct JSONB
/// query against `indexed_account` rather than a live `getProgramAccounts`
/// scan (see the module doc's "no RPC per request" contract) — the account
/// processor (src/processors/account.rs) keeps `owner`/`amount` current for
/// every position it observes, and `src/crawler.rs` deletes a position's row
/// entirely once `close_vote_position`/`close_tag_stake_position` actually
/// closes it, so a row showing up here is never already-closed.
async fn get_closeable_positions(
    State(state): State<Arc<ApiState>>,
    Path(owner): Path<String>,
) -> Result<Json<CloseablePositionsDto>, ApiError> {
    let owner_pk = parse_pubkey("owner", &owner)?;
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT pubkey, account_type, lamports
        FROM indexed_account
        WHERE account_type IN ('VotePosition', 'StakePosition')
          AND data->'data'->>'owner' = $1
          AND data->'data'->>'amount' = '0'
        ORDER BY pubkey
        "#,
    )
    .bind(owner_pk.to_string())
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;

    let positions = rows
        .into_iter()
        .map(|(pubkey, account_type, lamports)| CloseablePositionDto {
            position: pubkey,
            kind: if account_type == "VotePosition" { "vote" } else { "tagStake" },
            lamports,
        })
        .collect();
    Ok(Json(CloseablePositionsDto { positions }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimVoteReq {
    app_id: String,
    user: String,
}

async fn build_claim_vote_reward(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ClaimVoteReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    let user = parse_pubkey("user", &req.user)?;
    let app = app_pda(&state.program_id, &req.app_id);
    let position = vote_position_pda(&state.program_id, &app, &user);
    // A claim can only ever target a position on an app that genuinely
    // already exists — nothing to lazily create here.
    if !account_exists(&state.rpc, &app).await {
        return Err(not_found(format!("app {} does not exist on-chain yet", req.app_id)));
    }
    let config = config_pda(&state.program_id);
    let vault = vault_address(&config, &state.vote_token_mint);
    let user_token_account = associated_token_address(&user, &state.vote_token_mint);

    let data = CLAIM_VOTE_REWARD_DISC.to_vec();

    // `app` is read-only here (never mutated by claim_vote_reward — see
    // that handler's doc comment on ClaimVoteReward::app).
    let accounts = vec![
        AccountMeta::new_readonly(app, false),
        AccountMeta::new(position, false),
        AccountMeta::new_readonly(config, false),
        AccountMeta::new(vault, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new_readonly(user, true),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];
    // See `build_withdraw_vote`'s comment on the same pattern — a claim
    // implies a prior vote, which already required this ATA to exist, but
    // it could have been closed (emptied) in between.
    let instructions = vec![
        ata_create_idempotent_ix(&user, &user, &state.vote_token_mint),
        Instruction::new_with_bytes(state.program_id, &data, accounts),
    ];
    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimTagReq {
    app_id: String,
    tag_slug: String,
    user: String,
}

async fn build_claim_tag_reward(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ClaimTagReq>,
) -> Result<Json<BuiltTxDto>, ApiError> {
    validate_seed_len("app_id", &req.app_id, MAX_APP_ID_LEN)?;
    validate_seed_len("tag_slug", &req.tag_slug, MAX_TAG_ID_LEN)?;
    let user = parse_pubkey("user", &req.user)?;
    let app = app_pda(&state.program_id, &req.app_id);
    let tag = tag_pda(&state.program_id, &req.tag_slug);
    let app_tag_stake = app_tag_stake_pda(&state.program_id, &app, &tag);
    let position = stake_position_pda(&state.program_id, &app_tag_stake, &user);
    // A claim can only ever target stake that genuinely already exists
    // on-chain — nothing to lazily create here.
    if !account_exists(&state.rpc, &app).await {
        return Err(not_found(format!("app {} does not exist on-chain yet", req.app_id)));
    }
    if !account_exists(&state.rpc, &app_tag_stake).await {
        return Err(not_found(format!("tag {} does not exist on-chain yet", req.tag_slug)));
    }
    let config = config_pda(&state.program_id);
    let vault = vault_address(&config, &state.vote_token_mint);
    let user_token_account = associated_token_address(&user, &state.vote_token_mint);

    let data = CLAIM_TAG_REWARD_DISC.to_vec();

    // `app` and `app_tag_stake` are both read-only (see ClaimTagReward's doc
    // comments — neither field is ever mutated by this instruction).
    let accounts = vec![
        AccountMeta::new_readonly(app, false),
        AccountMeta::new_readonly(app_tag_stake, false),
        AccountMeta::new(position, false),
        AccountMeta::new_readonly(config, false),
        AccountMeta::new(vault, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new_readonly(user, true),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];
    // See `build_claim_vote_reward`'s comment on the same pattern.
    let instructions = vec![
        ata_create_idempotent_ix(&user, &user, &state.vote_token_mint),
        Instruction::new_with_bytes(state.program_id, &data, accounts),
    ];
    let tx = unsigned_tx_base64(&state.rpc, &user, instructions).await?;
    Ok(Json(BuiltTxDto { transaction: tx }))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum ClaimItem {
    Vote { app_id: String },
    Tag { app_id: String, tag_slug: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimAllReq {
    claims: Vec<ClaimItem>,
    user: String,
}

#[derive(Serialize)]
struct BuiltTxsDto {
    transactions: Vec<String>,
}

/// Solana's hard cap on a serialized transaction (`PACKET_DATA_SIZE`) —
/// signatures + message must fit in one UDP packet.
const MAX_TX_BYTES: usize = 1232;

/// Serialized size of `instructions` as a single-signer (`fee_payer`)
/// transaction, without an RPC round trip: a real blockhash is exactly 32
/// bytes like the placeholder one, and an unsigned `Transaction`'s signature
/// slots are already the right count/size (64-byte zeroed `Signature`s), so
/// this exactly matches what the eventual signed, real-blockhash transaction
/// will serialize to.
fn estimated_tx_size(instructions: &[Instruction], fee_payer: &Pubkey) -> usize {
    let message = Message::new_with_blockhash(instructions, Some(fee_payer), &Hash::default());
    let tx = Transaction::new_unsigned(message);
    bincode::serialize(&tx).map(|b| b.len()).unwrap_or(usize::MAX)
}

/// Builds the *minimum* number of transactions that pack every claim in
/// `req.claims`, so a caller can get every pending reward signed in one
/// `wallet.signAllTransactions()` prompt instead of one popup per claim (see
/// `ClaimRewards.tsx`'s "Claim all" button). Each claim contributes the same
/// idempotent-ATA-create + claim-instruction pair `build_claim_vote_reward`/
/// `build_claim_tag_reward` build individually; this greedily packs those
/// pairs into transactions up to Solana's `MAX_TX_BYTES` limit rather than
/// always starting a new one. The ATA-create instruction is repeated in
/// every transaction (not just the first) since transactions in a batch may
/// land in any order — repeating an idempotent instruction is harmless, an
/// absent one on a not-yet-created ATA is not.
///
/// Unlike the single-claim builders, this doesn't pre-check
/// `account_exists` for every claim (that would be one extra RPC call per
/// claim) — it trusts the caller to only pass positions it already loaded
/// from `/api/rewards/positions`; a stale/nonexistent one simply fails that
/// one transaction on-chain instead of the whole batch.
async fn build_claim_all_rewards(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ClaimAllReq>,
) -> Result<Json<BuiltTxsDto>, ApiError> {
    if req.claims.is_empty() {
        return Err(bad_request("No claims to build".to_string()));
    }
    let user = parse_pubkey("user", &req.user)?;
    let config = config_pda(&state.program_id);
    let vault = vault_address(&config, &state.vote_token_mint);
    let user_token_account = associated_token_address(&user, &state.vote_token_mint);
    let ata_ix = ata_create_idempotent_ix(&user, &user, &state.vote_token_mint);

    let mut per_claim_instructions: Vec<Vec<Instruction>> = Vec::with_capacity(req.claims.len());
    for claim in &req.claims {
        let (data, accounts) = match claim {
            ClaimItem::Vote { app_id } => {
                validate_seed_len("app_id", app_id, MAX_APP_ID_LEN)?;
                let app = app_pda(&state.program_id, app_id);
                let position = vote_position_pda(&state.program_id, &app, &user);
                (
                    CLAIM_VOTE_REWARD_DISC.to_vec(),
                    vec![
                        AccountMeta::new_readonly(app, false),
                        AccountMeta::new(position, false),
                        AccountMeta::new_readonly(config, false),
                        AccountMeta::new(vault, false),
                        AccountMeta::new(user_token_account, false),
                        AccountMeta::new_readonly(user, true),
                        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                    ],
                )
            }
            ClaimItem::Tag { app_id, tag_slug } => {
                validate_seed_len("app_id", app_id, MAX_APP_ID_LEN)?;
                validate_seed_len("tag_slug", tag_slug, MAX_TAG_ID_LEN)?;
                let app = app_pda(&state.program_id, app_id);
                let tag = tag_pda(&state.program_id, tag_slug);
                let app_tag_stake = app_tag_stake_pda(&state.program_id, &app, &tag);
                let position = stake_position_pda(&state.program_id, &app_tag_stake, &user);
                (
                    CLAIM_TAG_REWARD_DISC.to_vec(),
                    vec![
                        AccountMeta::new_readonly(app, false),
                        AccountMeta::new_readonly(app_tag_stake, false),
                        AccountMeta::new(position, false),
                        AccountMeta::new_readonly(config, false),
                        AccountMeta::new(vault, false),
                        AccountMeta::new(user_token_account, false),
                        AccountMeta::new_readonly(user, true),
                        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                    ],
                )
            }
        };
        per_claim_instructions.push(vec![
            ata_ix.clone(),
            Instruction::new_with_bytes(state.program_id, &data, accounts),
        ]);
    }

    let mut batches: Vec<Vec<Instruction>> = Vec::new();
    let mut current: Vec<Instruction> = Vec::new();
    for instructions in per_claim_instructions {
        let mut candidate = current.clone();
        candidate.extend(instructions.iter().cloned());
        if !current.is_empty() && estimated_tx_size(&candidate, &user) > MAX_TX_BYTES {
            batches.push(std::mem::take(&mut current));
            current = instructions;
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }

    let mut transactions = Vec::with_capacity(batches.len());
    for batch in batches {
        transactions.push(unsigned_tx_base64(&state.rpc, &user, batch).await?);
    }
    Ok(Json(BuiltTxsDto { transactions }))
}

// ---------------------------------------------------------------------
// Submission — the other RPC call this API makes: relaying an
// already-signed transaction (wallet signing happens client-side; the
// indexer never sees a private key). Works for both nebulous_world
// instructions and the DLMM swap built by the sidecar, since submission
// doesn't care about instruction content.
// ---------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitReq {
    signed_transaction: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmitDto {
    signature: String,
}

async fn submit_tx(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SubmitReq>,
) -> Result<Json<SubmitDto>, ApiError> {
    let bytes = BASE64
        .decode(req.signed_transaction)
        .map_err(|e| bad_request(format!("invalid base64: {e}")))?;
    let tx: Transaction =
        bincode::deserialize(&bytes).map_err(|e| bad_request(format!("invalid transaction: {e}")))?;
    let signature = state
        .rpc
        .send_and_confirm_transaction(&tx)
        .await
        .map_err(internal)?;
    Ok(Json(SubmitDto {
        signature: signature.to_string(),
    }))
}

// ---------------------------------------------------------------------
// Platform metrics history — the time series behind the Explore page's
// metric trend charts (src/platform_metrics.rs writes the rows this reads).
// ---------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformMetricsPointDto {
    captured_at: chrono::DateTime<chrono::Utc>,
    app_count: i64,
    tag_count: i64,
    /// Raw on-chain u64 amounts as decimal strings (see indexerClient.ts's
    /// convention) — the app scales these by the vote token's decimals.
    total_vote_stake: String,
    total_tag_stake: String,
}

async fn get_platform_metrics_history(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<Vec<PlatformMetricsPointDto>>, ApiError> {
    let rows: Vec<(chrono::DateTime<chrono::Utc>, i64, i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT captured_at, app_count, tag_count, total_vote_stake, total_tag_stake
        FROM (
            SELECT captured_at, app_count, tag_count, total_vote_stake, total_tag_stake
            FROM platform_metrics_snapshot
            WHERE captured_at > now() - make_interval(days => $1)
            ORDER BY captured_at DESC
            LIMIT 2000
        ) recent
        ORDER BY captured_at ASC
        "#,
    )
    .bind(crate::handlers::platform::PLATFORM_TREND_WINDOW_DAYS as i32)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;

    Ok(Json(
        rows.into_iter()
            .map(
                |(captured_at, app_count, tag_count, total_vote_stake, total_tag_stake)| {
                    PlatformMetricsPointDto {
                        captured_at,
                        app_count,
                        tag_count,
                        total_vote_stake: total_vote_stake.to_string(),
                        total_tag_stake: total_tag_stake.to_string(),
                    }
                },
            )
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-checked against app/src/lib/anchorClient.ts's appPda/
    /// votePositionPda/tagPda/appTagStakePda/stakePositionPda for the same
    /// (programId, appId, user, tagSlug) inputs — these two implementations
    /// must derive byte-identical PDAs, since the app signs against
    /// whichever address it independently computes, but only this API's
    /// derivation determines which accounts actually go into the built
    /// instruction.
    #[test]
    fn pda_derivation_matches_the_app_ts_client() {
        let program_id = Pubkey::from_str("7AAwB8aHgmuz9yzYdo6gKjZRx5KkjePYHyuJEmrcRXgX").unwrap();
        let app_id = "test-app-123";
        let user = Pubkey::from_str("5Hs51hUxpr9cBz8gwbsFNVcJqdtPeJR8MoUNxoUSGP8a").unwrap();
        let tag_slug = "example-tag";

        let config = config_pda(&program_id);
        let app = app_pda(&program_id, app_id);
        let vote_pos = vote_position_pda(&program_id, &app, &user);
        let tag = tag_pda(&program_id, tag_slug);
        let app_tag_stake = app_tag_stake_pda(&program_id, &app, &tag);
        let stake_pos = stake_position_pda(&program_id, &app_tag_stake, &user);

        assert_eq!(config.to_string(), "aSsmFCbhZeCtkk6jaqmtUHALDkBeYWkDVbcrEzsLaa5");
        assert_eq!(app.to_string(), "WFFEeYwFZEwqG6u39gmyeVdQpZZ2saJ1NdumT2ibf54");
        assert_eq!(vote_pos.to_string(), "5HyY7dqWi1xLjijrViq1Xm7dSSBfVFf4EwL26kLHGFL5");
        assert_eq!(tag.to_string(), "EwCP1sFK2Reuu4RiiwvygBffzCt8rEqxKocHFSTSenCF");
        assert_eq!(app_tag_stake.to_string(), "Amrp4xm898tGgeeXZ2zKoj2YEdfoRNF1NSFPw51ivLjR");
        assert_eq!(stake_pos.to_string(), "27N8WYexQynYkqkfP6vjts4ZCAd4KErdDqfEw2t77BcS");
    }

    /// Cross-checked against the real Associated Token Account program via
    /// `spl-token address --url mainnet-beta --token
    /// EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v --owner
    /// 11111111111111111111111111111112 --verbose`, which independently
    /// derives (and, via its live RPC call, confirms on-chain) this exact
    /// ATA for real USDC and the System Program's own address used as an
    /// arbitrary owner. This derivation must stay byte-identical to the
    /// canonical SPL implementation — `ata_create_idempotent_ix` below
    /// targets whatever address this computes, so a wrong derivation would
    /// silently create/address the wrong account rather than erroring.
    #[test]
    fn associated_token_address_matches_spl_token_cli() {
        let owner = Pubkey::from_str("11111111111111111111111111111112").unwrap();
        let mint = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let ata = associated_token_address(&owner, &mint);
        assert_eq!(ata.to_string(), "G9xKTRhM57AL4my3ZRVNqM95mxtACgKdNRPX6EVhB7hv");
    }

    /// The account order/mutability/discriminator this builds must match
    /// `@solana/spl-token`'s `createAssociatedTokenAccountIdempotentInstruction`
    /// exactly — cross-checked against
    /// app/node_modules/@solana/spl-token/lib/cjs/instructions/associatedTokenAccount.js's
    /// `buildAssociatedTokenAccountInstruction`. A wrong account order is
    /// silently accepted at the Rust type level (it's just a
    /// `Vec<AccountMeta>`) but rejected on-chain, so this locks the layout
    /// in as a regression test rather than relying on manual review alone.
    #[test]
    fn ata_create_idempotent_ix_matches_spl_token_layout() {
        let payer = Pubkey::from_str("5Hs51hUxpr9cBz8gwbsFNVcJqdtPeJR8MoUNxoUSGP8a").unwrap();
        let owner = Pubkey::from_str("11111111111111111111111111111112").unwrap();
        let mint = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let ix = ata_create_idempotent_ix(&payer, &owner, &mint);

        assert_eq!(ix.program_id, ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(ix.data, vec![1u8], "1 = CreateIdempotent discriminator, 0 would be Create (errors if it exists)");

        let expected_ata = associated_token_address(&owner, &mint);
        assert_eq!(ix.accounts.len(), 6);
        assert_eq!(ix.accounts[0], AccountMeta::new(payer, true), "payer: writable signer");
        assert_eq!(ix.accounts[1], AccountMeta::new(expected_ata, false), "associated_token: writable");
        assert_eq!(ix.accounts[2], AccountMeta::new_readonly(owner, false), "owner: readonly");
        assert_eq!(ix.accounts[3], AccountMeta::new_readonly(mint, false), "mint: readonly");
        assert_eq!(ix.accounts[4], AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false), "system_program: readonly");
        assert_eq!(ix.accounts[5], AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false), "token_program: readonly");
    }

    /// Regression test: an oversized app_id/tag_slug used to reach
    /// `Pubkey::find_program_address` unchecked, which panics rather than
    /// erroring on a >32-byte seed — confirmed live, it took down the
    /// in-flight request's connection outright. `validate_seed_len` must
    /// reject it with a clean 400 before any PDA derivation runs.
    #[test]
    fn validate_seed_len_rejects_oversized_input_without_panicking() {
        let ok = "a".repeat(32);
        assert!(validate_seed_len("app_id", &ok, MAX_APP_ID_LEN).is_ok());

        let too_long = "a".repeat(33);
        let err = validate_seed_len("app_id", &too_long, MAX_APP_ID_LEN);
        assert!(err.is_err());
    }
}
