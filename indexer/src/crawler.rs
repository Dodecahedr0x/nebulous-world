use crate::processors::account::delete_account;
use crate::processors::instruction::index_instruction;
use crate::processors::product;
use anyhow::Result;
use carbon_core::deserialize::ArrangeAccounts;
use carbon_core::instruction::{InstructionDecoder, InstructionMetadata};
use carbon_core::transaction::TransactionMetadata;
use carbon_nebulous_world_decoder::instructions::{
    CloseTagStakePosition, CloseVotePosition, NebulousWorldInstruction,
};
use carbon_nebulous_world_decoder::NebulousWorldDecoder;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_client::rpc_response::RpcConfirmedTransactionStatusWithSignature;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction_status::UiTransactionEncoding;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

const CURSOR_ID: &str = "program_instructions";
/// getSignaturesForAddress's hard per-call cap.
const SIGNATURES_PAGE_LIMIT: usize = 1000;

/// Capped exponential backoff for `with_retry` below — five attempts spread
/// over ~15s (500ms, 1s, 2s, 4s, 8s), long enough to ride out a rate-limit
/// window without stalling a single tick for multiple poll intervals.
const MAX_RETRIES: u32 = 5;
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);

/// Retries `f` with capped exponential backoff. Every RPC call this crawler
/// makes (`getSignaturesForAddress`, `getTransaction`) goes through a local
/// Surfnet that transparently proxies to the public `mainnet-beta` RPC for
/// any address it doesn't already have full local history for (see this
/// module's doc comment on why a fork is used at all) — and that public
/// endpoint's rate limit is easily tripped by this app's own local-dev setup
/// (scripts/createAppsOnchain.ts, scripts/seedStakes.ts each send hundreds
/// of transactions in parallel). Without a retry, a single 429 mid-tick used
/// to abort the whole tick via `?` *before* the cursor was ever saved — so
/// nothing landed in Postgres (`App`/`Tag` rows only ever come from this
/// crawler, see processors/product.rs) until a tick happened to land with
/// zero contention, which wasn't guaranteed to ever happen while the setup
/// script's own burst was still running.
async fn with_retry<T, F, Fut>(mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut backoff = INITIAL_BACKOFF;
    for attempt in 1..=MAX_RETRIES {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt < MAX_RETRIES => {
                log::warn!(
                    "crawler: RPC call failed (attempt {attempt}/{MAX_RETRIES}), retrying in {backoff:?}: {e}"
                );
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("loop always returns on the final attempt")
}

/// Polls `getSignaturesForAddress` + `getTransaction` for "our program
/// changes" instead of the Carbon-provided `RpcBlockSubscribe` datasource:
/// `blockSubscribe` is disabled by default on `solana-test-validator` and,
/// more importantly, on essentially every hosted RPC provider including the
/// public devnet endpoint this app deploys against (see render.yaml) — a
/// live-streaming pipeline built on it would silently index nothing in
/// production. `getSignaturesForAddress`/`getTransaction` are universally
/// supported standard RPC methods, at the cost of polling latency instead
/// of push updates.
///
/// Deliberately hand-rolled rather than using
/// `carbon-rpc-transaction-crawler-datasource` (which solves this exact
/// problem): that crate is only published against carbon-core 1.0.0, which
/// has breaking changes the carbon-cli-generated decoder (pinned to 0.12.0
/// throughout this workspace — see decoder/Cargo.toml) isn't compatible
/// with, and carbon-cli itself hasn't shipped a 1.0.0-targeting release yet
/// to regenerate against. Scoped down accordingly: only top-level
/// instructions are decoded (this program is never invoked via CPI in this
/// app), and only `VersionedMessage::static_account_keys()` is used to
/// resolve account metas — an instruction referencing an
/// address-lookup-table-loaded account (never done by this app's own
/// transaction-building code) is skipped entirely rather than indexed with
/// wrong/shifted account roles.
///
/// The cursor (`crawler_cursor` table) persists the newest fully-processed
/// signature so a restart resumes instead of re-scanning; each tick also
/// paginates via `before` until it has fetched everything newer than the
/// cursor, so a burst of more than `SIGNATURES_PAGE_LIMIT` (1000)
/// signatures between polls can't silently drop the older ones the way a
/// single un-paginated call would.
pub async fn run(
    rpc_http_url: String,
    program_id: Pubkey,
    pool: PgPool,
    poll_interval_secs: u64,
    http: reqwest::Client,
) {
    let client = RpcClient::new(rpc_http_url);
    let decoder = NebulousWorldDecoder;
    let mut ticker = tokio::time::interval(Duration::from_secs(poll_interval_secs));

    loop {
        ticker.tick().await;
        if let Err(e) = crawl_once(&client, &decoder, program_id, &pool, &http).await {
            log::error!("crawler: tick failed: {e}");
        }
    }
}

async fn load_cursor(pool: &PgPool) -> Result<Option<Signature>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT last_signature FROM crawler_cursor WHERE id = $1")
            .bind(CURSOR_ID)
            .fetch_optional(pool)
            .await?;
    Ok(match row {
        Some((sig,)) => Some(sig.parse()?),
        None => None,
    })
}

async fn save_cursor(pool: &PgPool, signature: Signature) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO crawler_cursor (id, last_signature, updated_at)
        VALUES ($1, $2, now())
        ON CONFLICT (id) DO UPDATE SET last_signature = EXCLUDED.last_signature, updated_at = now()
        "#,
    )
    .bind(CURSOR_ID)
    .bind(signature.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetches every signature newer than `until`, paginating via `before` so a
/// burst larger than one page can't be silently truncated.
async fn fetch_new_signatures(
    client: &RpcClient,
    program_id: Pubkey,
    until: Option<Signature>,
) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>> {
    let mut all = Vec::new();
    let mut before: Option<Signature> = None;
    loop {
        let batch = with_retry(|| async {
            client
                .get_signatures_for_address_with_config(
                    &program_id,
                    GetConfirmedSignaturesForAddress2Config {
                        before,
                        until,
                        limit: Some(SIGNATURES_PAGE_LIMIT),
                        commitment: Some(CommitmentConfig::confirmed()),
                    },
                )
                .await
                .map_err(Into::into)
        })
        .await?;
        let is_full_page = batch.len() == SIGNATURES_PAGE_LIMIT;
        let Some(oldest) = batch.last() else { break };
        before = Some(oldest.signature.parse()?);
        all.extend(batch);
        if !is_full_page {
            break;
        }
    }
    Ok(all)
}

async fn crawl_once(
    client: &RpcClient,
    decoder: &NebulousWorldDecoder,
    program_id: Pubkey,
    pool: &PgPool,
    http: &reqwest::Client,
) -> Result<()> {
    let until = load_cursor(pool).await?;
    let signatures = fetch_new_signatures(client, program_id, until).await?;
    if signatures.is_empty() {
        return Ok(());
    }
    // getSignaturesForAddress returns newest-first; process oldest-first so
    // indexed_instruction fills in chronological order.
    let newest = signatures[0].signature.parse::<Signature>()?;

    for entry in signatures.into_iter().rev() {
        if entry.err.is_some() {
            continue; // failed transactions never touched account state
        }
        let signature = entry.signature.parse::<Signature>()?;
        if let Err(e) =
            crawl_transaction(client, decoder, program_id, pool, signature, entry.slot, http).await
        {
            log::error!("crawler: failed to process {signature}: {e}");
        }
    }

    save_cursor(pool, newest).await?;
    Ok(())
}

async fn crawl_transaction(
    client: &RpcClient,
    decoder: &NebulousWorldDecoder,
    program_id: Pubkey,
    pool: &PgPool,
    signature: Signature,
    slot: u64,
    http: &reqwest::Client,
) -> Result<()> {
    let response = with_retry(|| async {
        client
            .get_transaction_with_config(
                &signature,
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Base64),
                    commitment: Some(CommitmentConfig::confirmed()),
                    max_supported_transaction_version: Some(0),
                },
            )
            .await
            .map_err(Into::into)
    })
    .await?;

    let Some(versioned_tx) = response.transaction.transaction.decode() else {
        log::warn!("crawler: could not decode transaction {signature}, skipping");
        return Ok(());
    };
    let account_keys = versioned_tx.message.static_account_keys();
    let block_time = response.block_time;

    // A companion SPL Memo instruction carries the crowd-submitted app
    // metadata that has no on-chain `AppAccount` field of its own (name,
    // tagline, description, ...) — see processors/product.rs's doc comment.
    // Scanned once per transaction rather than per-instruction below since
    // it's cheap and only ever consumed by `init_app`.
    let memo_text = versioned_tx
        .message
        .instructions()
        .iter()
        .find_map(|compiled| {
            let program_id = account_keys.get(compiled.program_id_index as usize)?;
            if !product::is_memo_program(program_id) {
                return None;
            }
            Some(String::from_utf8_lossy(&compiled.data).into_owned())
        });

    // A minimal `TransactionMetadata` — only the fields `index_instruction`
    // actually reads (signature, slot, block_time) are populated; the rest
    // carry harmless defaults since nothing downstream of this hand-rolled
    // path consults them.
    let transaction_metadata = Arc::new(TransactionMetadata {
        slot,
        signature,
        fee_payer: account_keys.first().copied().unwrap_or_default(),
        meta: solana_transaction_status::TransactionStatusMeta::default(),
        message: versioned_tx.message.clone(),
        block_time,
        block_hash: None,
    });

    for (index, compiled) in versioned_tx.message.instructions().iter().enumerate() {
        let Some(&ix_program_id) = account_keys.get(compiled.program_id_index as usize) else {
            continue;
        };
        if ix_program_id != program_id {
            continue;
        }

        // Every account index must resolve against the STATIC keys list —
        // an address-lookup-table-loaded account (never used by this app's
        // own transaction-building code) can't be, and silently dropping
        // just that account via filter_map would shift every subsequent
        // account into the wrong role. Skip the whole instruction instead.
        let Some(accounts) = compiled
            .accounts
            .iter()
            .map(|&i| {
                account_keys.get(i as usize).map(|&pubkey| AccountMeta {
                    pubkey,
                    is_signer: false,
                    is_writable: false,
                })
            })
            .collect::<Option<Vec<_>>>()
        else {
            log::warn!(
                "crawler: instruction {index} in {signature} references an account outside the static keys list (likely an ALT-loaded account), skipping"
            );
            continue;
        };
        let instruction = Instruction {
            program_id: ix_program_id,
            accounts,
            data: compiled.data.clone(),
        };

        let Some(decoded) = decoder.decode_instruction(&instruction) else {
            continue;
        };
        let metadata = InstructionMetadata {
            transaction_metadata: transaction_metadata.clone(),
            stack_height: 1,
            index: index as u32,
            absolute_path: vec![index as u8],
        };

        if let Err(e) = index_instruction(pool, &metadata, &decoded).await {
            log::error!("crawler: failed to index instruction in {signature}: {e}");
        }

        // `App`/`Tag`/`AppTag` are populated exclusively from here — see
        // processors/product.rs's doc comment for why this, rather than a
        // seed script or an app-owned write path, is "the database" for
        // these tables. Failures are logged, not propagated: one
        // malformed/unexpected transaction shouldn't stop the whole batch
        // from being marked processed (the cursor still advances past it).
        match &decoded.data {
            NebulousWorldInstruction::InitApp(init_app) => {
                match product::init_app_payer(&decoded.accounts) {
                    Some(payer) => {
                        if let Err(e) = product::sync_app_from_init(
                            pool,
                            http,
                            init_app,
                            &payer,
                            memo_text.as_deref(),
                        )
                        .await
                        {
                            log::error!("crawler: failed to sync App from {signature}: {e}");
                        }
                    }
                    None => log::warn!(
                        "crawler: init_app in {signature} has an unexpected account layout, skipping App sync"
                    ),
                }
            }
            NebulousWorldInstruction::SuggestTag(suggest_tag) => {
                match product::suggest_tag_payer(&decoded.accounts) {
                    Some(payer) => {
                        if let Err(e) =
                            product::sync_tag_from_suggest(pool, suggest_tag, &payer).await
                        {
                            log::error!("crawler: failed to sync Tag from {signature}: {e}");
                        }
                    }
                    None => log::warn!(
                        "crawler: suggest_tag in {signature} has an unexpected account layout, skipping Tag sync"
                    ),
                }
            }
            // The one case the live account-update pipeline (src/processors/account.rs)
            // can't observe on its own: a closed account stops being
            // owned by this program, so it simply drops out of that
            // subscription's filter instead of producing a final "gone"
            // update — this decoded instruction is the only signal we get
            // that the row must be removed rather than upserted.
            NebulousWorldInstruction::CloseVotePosition(_) => {
                match CloseVotePosition::arrange_accounts(&decoded.accounts) {
                    Some(accounts) => {
                        if let Err(e) =
                            delete_account(pool, &accounts.position.to_string()).await
                        {
                            log::error!(
                                "crawler: failed to delete closed VotePosition from {signature}: {e}"
                            );
                        }
                    }
                    None => log::warn!(
                        "crawler: close_vote_position in {signature} has an unexpected account layout, skipping cleanup"
                    ),
                }
            }
            NebulousWorldInstruction::CloseTagStakePosition(_) => {
                match CloseTagStakePosition::arrange_accounts(&decoded.accounts) {
                    Some(accounts) => {
                        if let Err(e) =
                            delete_account(pool, &accounts.position.to_string()).await
                        {
                            log::error!(
                                "crawler: failed to delete closed StakePosition from {signature}: {e}"
                            );
                        }
                    }
                    None => log::warn!(
                        "crawler: close_tag_stake_position in {signature} has an unexpected account layout, skipping cleanup"
                    ),
                }
            }
            _ => {}
        }
    }

    Ok(())
}
