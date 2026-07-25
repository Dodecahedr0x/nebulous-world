// Reads a JSON file of {url, name?, tagline?, description?, iconUrl?,
// category?, chain?, tags?} entries (default: scripts/appData/apps.json)
// and creates each one on-chain: one `init_app` (+ one `suggest_tag` per
// initial tag, + a leading Memo instruction carrying whatever metadata has
// no on-chain field of its own) transaction per app — the exact same
// instructions and shape app/src/hooks/useCreateAppProgram.ts builds for a
// user's own wallet-signed "Create app" flow (see indexer/src/api.rs's
// build_create_app), just signed by a local keypair here instead of a
// browser wallet.
//
// This is NOT a database seed script (see AGENTS.md: there isn't one, and
// there must never be one) — nothing here touches Postgres. The `App`/
// `Tag`/`AppTag` rows only start existing once the indexer observes these
// transactions confirmed on-chain, exactly like any other app creation.
//
// Idempotent: `app_id` is derived deterministically from each entry's URL
// (sha256), so re-running only ever creates whichever apps don't already
// exist on-chain yet — safe to run on every `setup:dev` / every deploy.
//
// Usage:
//   tsx --env-file=.env scripts/createAppsOnchain.ts [--file=scripts/appData/apps.json] [--limit=N] [--concurrency=N] [--dry-run]

import { readFileSync } from "fs";
import { homedir } from "os";
import { createHash } from "crypto";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { AnchorProvider, Program, Wallet } from "@anchor-lang/core";
import { appPda, tagPda, appTagStakePda } from "../src/lib/anchorClient";
import { slugify } from "../src/lib/utils";
import { CATEGORIES, CHAINS } from "../src/lib/constants";
import { config } from "../src/lib/config";
import { mapWithConcurrency } from "./lib/concurrency";
import { parseFlags } from "./lib/parseFlags";
import { withRateLimitRetry } from "./lib/retry";
import idl from "../../target/idl/nebulous_world.json";
import type { NebulousWorld } from "../../target/types/nebulous_world";

// Matches setup-dev.sh's DEV_KEYPAIR by default — the same funded local
// wallet everything else in local dev uses. Override with
// DEPLOYER_KEYPAIR_PATH for a real deployment (a funded devnet/mainnet
// keypair; this script is never run automatically as part of a Render
// build — see README's deployment runbook).
const DEPLOYER_KEYPAIR_PATH =
  process.env.DEPLOYER_KEYPAIR_PATH || `${homedir()}/.config/solana/id.json`;

// SPL Memo v2 — same program id indexer/src/api.rs's `memo_ix` targets.
const MEMO_PROGRAM_ID = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

// Mirrors programs/nebulous_world/src/constants.rs's MAX_APP_ID_LEN/
// MAX_TAG_ID_LEN (both 32 bytes — PDA seeds are capped at 32 bytes each).
const MAX_ID_LEN = 32;
// Mirrors MAX_URL_LEN.
const MAX_URL_LEN = 200;
// Keeps the initial creation transaction comfortably under Solana's
// ~1232-byte limit — init_app + a memo + this many suggest_tag instructions
// (each needs 2 fresh accounts) fits with room to spare. This is not a cap
// on how many tags an app can have: any tags beyond this bundle into the
// atomic create tx, and the rest go out right after as one suggest_tag
// transaction each (same shape as TagStakePanel's "add a tag" flow), so an
// app ends up with every tag from its entry regardless of count.
const TAGS_PER_TX = 5;

export interface AppEntry {
  url: string;
  name?: string;
  tagline?: string;
  description?: string;
  iconUrl?: string;
  category?: string;
  chain?: string;
  tags?: string[];
}

interface Args {
  file: string;
  limit: number;
  concurrency: number;
  dryRun: boolean;
}

function parseArgs(argv: string[]): Args {
  const f = parseFlags(argv);
  return {
    file: typeof f.file === "string" ? f.file : "scripts/appData/apps.json",
    limit: typeof f.limit === "string" ? Number(f.limit) : Infinity,
    concurrency: typeof f.concurrency === "string" ? Number(f.concurrency) : 3,
    dryRun: Boolean(f["dry-run"]),
  };
}

/** Deterministic so re-running this script targets the exact same on-chain PDA for a given URL. */
export function deriveAppId(url: string): string {
  return createHash("sha256").update(url.trim().toLowerCase()).digest("hex").slice(0, MAX_ID_LEN);
}

function trimProtocol(url: string): string {
  return url.trim().replace(/^https?:\/\//, "");
}

/**
 * Metadata with no on-chain `AppAccount` field of its own (name/tagline/
 * description/iconUrl/category/chain) — carried as a Memo instruction, same
 * shape indexer/src/processors/product.rs's `AppMemo` parses (plain
 * snake_case field names, no camelCase rename). `null` if the entry has
 * nothing worth attaching.
 */
function memoInstruction(entry: AppEntry, payer: PublicKey): TransactionInstruction | null {
  const memo: Record<string, string> = {};
  if (entry.name) memo.name = entry.name;
  if (entry.tagline) memo.tagline = entry.tagline;
  if (entry.description) memo.description = entry.description;
  if (entry.iconUrl) memo.icon_url = entry.iconUrl;
  if (entry.category && (CATEGORIES as readonly string[]).includes(entry.category)) memo.category = entry.category;
  if (entry.chain && (CHAINS as readonly string[]).includes(entry.chain)) memo.chain = entry.chain;
  if (Object.keys(memo).length === 0) return null;
  return new TransactionInstruction({
    keys: [{ pubkey: payer, isSigner: true, isWritable: false }],
    programId: MEMO_PROGRAM_ID,
    data: Buffer.from(JSON.stringify(memo), "utf-8"),
  });
}

type CreateResult =
  | { status: "created"; appId: string; sig: string }
  | { status: "skipped"; appId: string }
  | { status: "dry-run"; appId: string }
  | { status: "failed"; appId: string; error: string };

async function createApp(
  program: Program<NebulousWorld>,
  provider: AnchorProvider,
  payer: Keypair,
  entry: AppEntry,
  dryRun: boolean,
): Promise<CreateResult> {
  const appId = deriveAppId(entry.url);
  const app = appPda(program.programId, appId);
  const label = entry.name ?? entry.url;

  if (await withRateLimitRetry(() => provider.connection.getAccountInfo(app))) {
    return { status: "skipped", appId };
  }
  if (dryRun) return { status: "dry-run", appId };

  try {
    const instructions: TransactionInstruction[] = [];
    const memoIx = memoInstruction(entry, payer.publicKey);
    if (memoIx) instructions.push(memoIx);

    instructions.push(
      await program.methods
        .initApp(appId, trimProtocol(entry.url).slice(0, MAX_URL_LEN))
        .accountsPartial({ app, payer: payer.publicKey, systemProgram: SystemProgram.programId })
        .instruction(),
    );

    const tags = [...new Set((entry.tags ?? []).map((t) => slugify(t).slice(0, MAX_ID_LEN)).filter(Boolean))];
    const initialTags = tags.slice(0, TAGS_PER_TX);
    const extraTags = tags.slice(TAGS_PER_TX);

    for (const tagId of initialTags) {
      const tag = tagPda(program.programId, tagId);
      const appTagStake = appTagStakePda(program.programId, app, tag);
      instructions.push(
        await program.methods
          .suggestTag(appId, tagId)
          .accountsPartial({
            app,
            tag,
            appTagStake,
            payer: payer.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .instruction(),
      );
    }

    const tx = new Transaction().add(...instructions);
    const sig = await withRateLimitRetry(() => provider.sendAndConfirm(tx, [payer]));

    if (extraTags.length > 0) {
      console.log(`  … ${label}: sending ${extraTags.length} additional tag(s) as separate transactions`);
    }
    for (const tagId of extraTags) {
      const tag = tagPda(program.programId, tagId);
      const appTagStake = appTagStakePda(program.programId, app, tag);
      const extraIx = await program.methods
        .suggestTag(appId, tagId)
        .accountsPartial({
          app,
          tag,
          appTagStake,
          payer: payer.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      await withRateLimitRetry(() => provider.sendAndConfirm(new Transaction().add(extraIx), [payer]));
    }

    return { status: "created", appId, sig };
  } catch (err) {
    return { status: "failed", appId, error: err instanceof Error ? err.message : String(err) };
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));

  const raw = readFileSync(args.file, "utf-8");
  const entries = (JSON.parse(raw) as AppEntry[]).slice(0, args.limit);
  console.log(`📦 ${entries.length} app(s) from ${args.file}`);

  if (!config.solana.programId) {
    throw new Error("NEXT_PUBLIC_NEBULOUS_WORLD_PROGRAM_ID must be set");
  }

  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(DEPLOYER_KEYPAIR_PATH, "utf-8"))),
  );
  const connection = new Connection(config.solana.rpc, "confirmed");
  const provider = new AnchorProvider(connection, new Wallet(payer), { commitment: "confirmed" });
  const program = new Program<NebulousWorld>(idl as NebulousWorld, provider);

  console.log(`🔑 Paying/signing with ${payer.publicKey.toBase58()}`);
  if (args.dryRun) console.log("🧪 Dry run — no transactions will be sent");

  const results = await mapWithConcurrency(entries, args.concurrency, (entry) =>
    createApp(program, provider, payer, entry, args.dryRun),
  );

  let created = 0;
  let skipped = 0;
  let failed = 0;
  results.forEach((result, i) => {
    const label = entries[i]!.name ?? entries[i]!.url;
    if (result.status === "created") {
      created++;
      console.log(`  ✓ ${label} (${result.appId.slice(0, 8)}…, tx ${result.sig.slice(0, 12)}…)`);
    } else if (result.status === "skipped") {
      skipped++;
      console.log(`  – ${label}: already exists on-chain, skipping`);
    } else if (result.status === "failed") {
      failed++;
      console.error(`  ✗ ${label}: ${result.error}`);
    }
  });

  console.log(
    `${args.dryRun ? "🧪 Dry run — " : ""}${created} created, ${skipped} already existed, ${failed} failed.`,
  );
  if (failed > 0 && !args.dryRun) process.exitCode = 1;
}

if (require.main === module) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
