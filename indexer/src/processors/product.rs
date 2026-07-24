//! Populates the product's own tables (`App`, `Tag`, `AppTag`, `User` — the
//! tables `app/prisma/schema.prisma` describes, whose DDL now lives in
//! `indexer/migrations/005_app_schema.sql`, see that file's doc comment)
//! from confirmed on-chain instructions. This is the ONLY writer of `App`/
//! `Tag`/`AppTag` rows — there is no app-owned "create app" endpoint and no
//! seed script (see AGENTS.md). Called from `src/crawler.rs`, which already
//! replays full program history on a fresh database (crawler_cursor starts
//! empty) before settling into live polling, so this naturally backfills on
//! startup and stays current afterward. `src/reconcile.rs` runs alongside
//! this as a startup-only safety net — it can add a placeholder `App`/`Tag`
//! row from the raw account snapshot alone (no memo metadata) if this path
//! somehow missed one, but this crawler-driven path is still the only
//! source of real app/tag metadata.
//!
//! `Vote`/`Stake`/`Ad`/`RevenueEpoch`/... keep their existing app-owned
//! write paths (recorded by the Next.js API once a wallet-signed transaction
//! confirms) — only app/tag *creation* and schema ownership moved here.

use carbon_core::deserialize::ArrangeAccounts;
use carbon_nebulous_world_decoder::instructions::{InitApp, SuggestTag};
use serde::Deserialize;
use solana_pubkey::Pubkey;
use sqlx::PgPool;

/// Optional rich metadata a client can attach to an `init_app` transaction
/// via a companion SPL Memo instruction (name/tagline/description/etc. have
/// no on-chain field of their own — see `AppAccount`'s doc comment — so this
/// is the only way for a crowd-submitted app to carry them through to
/// Postgres without an app-owned write path). Every field is optional:
/// missing or unparseable memo data just means the app is created with
/// placeholder metadata, never that creation fails — the on-chain
/// transaction is the thing that must succeed, not the memo.
#[derive(Debug, Default, Deserialize)]
struct AppMemo {
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

/// Well-known SPL Memo program ids (v2 is what current wallet-adapter/web3.js
/// tooling emits; v1 is accepted too since nothing about parsing it is extra
/// work). Not `#[constant]`-derived from the program's own IDL because this
/// is a different, standard Solana program, not `nebulous_world` itself.
const MEMO_V2_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
const MEMO_V1_PROGRAM_ID: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";

pub fn is_memo_program(program_id: &Pubkey) -> bool {
    let s = program_id.to_string();
    s == MEMO_V2_PROGRAM_ID || s == MEMO_V1_PROGRAM_ID
}

fn parse_memo(memo_text: Option<&str>) -> AppMemo {
    memo_text
        .and_then(|text| serde_json::from_str::<AppMemo>(text).ok())
        .unwrap_or_default()
}

/// Same rules as `app/src/lib/utils.ts`'s `slugify` (lowercase, strip quotes,
/// collapse runs of non-alphanumeric characters to a single `-`, trim
/// leading/trailing `-`, cap at 80 bytes) — kept in sync by hand since this
/// is the only other place a slug is derived from free text.
fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in input.chars() {
        if ch == '\'' || ch == '"' {
            continue;
        }
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(lower);
        } else {
            pending_dash = true;
        }
    }
    out.truncate(80);
    out.trim_end_matches('-').to_string()
}

/// Upserts a `User` row keyed by wallet, returning its id. `User.id` has no
/// database-level default (Prisma's `@default(cuid())` is applied by Prisma
/// Client, not the database — see `005_app_schema.sql`'s header), so a
/// freshly-created row uses the wallet address itself as `id`; an
/// already-existing row (e.g. created earlier through the app's own
/// sign-in-with-Solana flow, which does use a Prisma cuid) keeps its
/// existing id — `RETURNING id` reflects whichever actually applies.
async fn upsert_user_by_wallet(pool: &PgPool, wallet: &str) -> anyhow::Result<String> {
    let (id,): (String,) = sqlx::query_as(
        r#"
        INSERT INTO "User" (id, wallet, "createdAt", "updatedAt")
        VALUES ($1, $1, now(), now())
        ON CONFLICT (wallet) DO UPDATE SET wallet = EXCLUDED.wallet
        RETURNING id
        "#,
    )
    .bind(wallet)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Creates the `App` row for a confirmed `init_app` instruction. `app_id`
/// becomes `App.id` directly (see the Prisma schema's doc comment on `App`);
/// `url` is `AppAccount.url` with `https://` prepended back on (it was
/// trimmed off before being stored on-chain — see `indexer/src/api.rs`'s
/// `init_app_ix`). `ON CONFLICT DO NOTHING`: this instruction can only ever
/// succeed once on-chain for a given `app_id` (the account `init` constraint
/// enforces that), so a second sighting only happens if the crawler
/// reprocesses a signature after a restart before its cursor advanced —
/// a harmless no-op, not a real update.
pub async fn sync_app_from_init(
    pool: &PgPool,
    http: &reqwest::Client,
    decoded: &InitApp,
    payer: &Pubkey,
    memo_text: Option<&str>,
) -> anyhow::Result<()> {
    let memo = parse_memo(memo_text);
    let submitted_by = upsert_user_by_wallet(pool, &payer.to_string()).await?;

    let url = format!("https://{}", decoded.url);
    let name = memo
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| decoded.url.clone());
    let slug_base = slugify(&name);
    let slug = if slug_base.is_empty() {
        decoded.app_id.clone()
    } else {
        slug_base
    };
    let tagline = memo.tagline.unwrap_or_default();
    let description = memo.description.unwrap_or_default();
    let icon_url = memo.icon_url.filter(|s| !s.trim().is_empty());
    let category = memo
        .category
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "other".to_string());
    let chain = memo
        .chain
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "solana".to_string());

    let result = sqlx::query(
        r#"
        INSERT INTO "App"
            (id, slug, name, tagline, description, url, "iconUrl", category, chain,
             status, "submittedBy", "createdAt", "updatedAt")
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'approved', $10, now(), now())
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(&decoded.app_id)
    .bind(&slug)
    .bind(&name)
    .bind(&tagline)
    .bind(&description)
    .bind(&url)
    .bind(&icon_url)
    .bind(&category)
    .bind(&chain)
    .bind(&submitted_by)
    .execute(pool)
    .await?;

    log::info!("synced App {} (slug {slug}) from init_app", decoded.app_id);

    // Fetch the app's own OpenGraph metadata in the background to fill in
    // whichever of icon/tagline/description the memo didn't already supply
    // — see opengraph.rs's doc comment. Gated on `rows_affected() > 0` (a
    // genuinely fresh row, not a re-seen signature on a crawler restart/
    // replay) so re-processing already-synced history never re-fetches
    // OpenGraph data for apps that already have it.
    if result.rows_affected() > 0 {
        crate::opengraph::spawn_enrichment(
            pool.clone(),
            http.clone(),
            decoded.app_id.clone(),
            url,
            icon_url.is_none(),
            tagline.is_empty(),
            description.is_empty(),
        );
    }

    if let Err(e) = crate::handlers::xp::award(
        pool,
        &submitted_by,
        "submit_app",
        Some(&decoded.app_id),
        crate::handlers::xp::XP_SUBMIT_APP,
    )
    .await
    {
        log::warn!("failed to award submit_app XP for user {submitted_by}: {e}");
    }

    Ok(())
}

/// Fills in whichever of icon_url/tagline/description is still missing on
/// an existing `App` row — never overwrites an already-set field. Shared by
/// `PATCH /apps/:id/metadata` (src/handlers/apps.rs, a manual/scripted
/// catch-up path) and the automatic post-creation enrichment
/// `sync_app_from_init` above triggers (src/opengraph.rs).
pub async fn apply_metadata_update(
    pool: &PgPool,
    app_id: &str,
    icon_url: Option<&str>,
    tagline: Option<&str>,
    description: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE "App" SET
            "iconUrl" = COALESCE($2, "iconUrl"),
            tagline = COALESCE(NULLIF($3, ''), tagline),
            description = COALESCE(NULLIF($4, ''), description),
            "updatedAt" = now()
        WHERE id = $1
        "#,
    )
    .bind(app_id)
    .bind(icon_url)
    .bind(tagline)
    .bind(description)
    .execute(pool)
    .await?;
    Ok(())
}

/// Creates/reuses the global `Tag` row and creates the `AppTag` link for a
/// confirmed `suggest_tag` instruction. `Tag.id`/`AppTag.id` have no
/// database-level default either (same reasoning as `App.id`) — `Tag` reuses
/// the on-chain `tag_id` directly, `AppTag` uses a deterministic
/// `"{appId}_{tagId}"` so re-processing the same instruction stays a no-op
/// via `ON CONFLICT` rather than needing a prior lookup.
pub async fn sync_tag_from_suggest(
    pool: &PgPool,
    decoded: &SuggestTag,
    payer: &Pubkey,
) -> anyhow::Result<()> {
    let suggested_by = upsert_user_by_wallet(pool, &payer.to_string()).await?;
    let tag_name = decoded.tag_id.replace('-', " ");
    let tag_slug = slugify(&decoded.tag_id);
    let tag_slug = if tag_slug.is_empty() {
        decoded.tag_id.clone()
    } else {
        tag_slug
    };

    sqlx::query(
        r#"
        INSERT INTO "Tag" (id, slug, name, "createdAt")
        VALUES ($1, $2, $3, now())
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(&decoded.tag_id)
    .bind(&tag_slug)
    .bind(&tag_name)
    .execute(pool)
    .await?;

    let app_tag_id = format!("{}_{}", decoded.app_id, decoded.tag_id);
    sqlx::query(
        r#"
        INSERT INTO "AppTag" (id, "appId", "tagId", "suggestedBy", "createdAt", "stakeTotal")
        VALUES ($1, $2, $3, $4, now(), 0)
        ON CONFLICT ("appId", "tagId") DO NOTHING
        "#,
    )
    .bind(&app_tag_id)
    .bind(&decoded.app_id)
    .bind(&decoded.tag_id)
    .bind(&suggested_by)
    .execute(pool)
    .await?;

    log::info!(
        "synced Tag {} + AppTag {app_tag_id} from suggest_tag",
        decoded.tag_id
    );

    if let Err(e) = crate::handlers::xp::award(
        pool,
        &suggested_by,
        "suggest_tag",
        Some(&app_tag_id),
        crate::handlers::xp::XP_SUGGEST_TAG,
    )
    .await
    {
        log::warn!("failed to award suggest_tag XP for user {suggested_by}: {e}");
    }

    Ok(())
}

/// Extracts the payer pubkey from `init_app`'s arranged accounts. Returns
/// `None` (logged by the caller) rather than erroring: a malformed/
/// unexpected account list here would otherwise take down the whole crawl
/// tick for every other instruction in the batch.
pub fn init_app_payer(accounts: &[solana_instruction::AccountMeta]) -> Option<Pubkey> {
    InitApp::arrange_accounts(accounts).map(|a| a.payer)
}

pub fn suggest_tag_payer(accounts: &[solana_instruction::AccountMeta]) -> Option<Pubkey> {
    SuggestTag::arrange_accounts(accounts).map(|a| a.payer)
}
