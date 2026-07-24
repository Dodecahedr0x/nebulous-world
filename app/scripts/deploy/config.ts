// Config schema + loader for the end-to-end production deploy script (see
// ./index.ts). Mirrors launch-neb/config.ts's JSONC-file pattern — too many
// interdependent steps/knobs for env vars or CLI flags alone. See
// deploy.config.example.jsonc.

import { readFileSync } from "fs";
import { homedir } from "os";
import { parse as parseJsonc } from "jsonc-parser";
import { z } from "zod";
import { tokenSchema, poolSchema } from "../launch-neb/config";

function expandHome(path: string): string {
  return path.startsWith("~") ? path.replace(/^~/, homedir()) : path;
}

const pubkeySchema = z.string().min(32).max(64);
// Optional fields ship in the example config as "" placeholders (not
// omitted) — treat empty string the same as omitted rather than failing
// pubkeySchema's min-length check on an unfilled-in template value.
const optionalString = z.preprocess((v) => (v === "" ? undefined : v), z.string().optional());
const optionalPubkey = z.preprocess((v) => (v === "" ? undefined : v), pubkeySchema.optional());

// Anchor's own cluster monikers (see Anchor.toml's [programs.*] sections and
// `anchor deploy --provider.cluster`) — "mainnet", not "mainnet-beta".
export function toAnchorCluster(cluster: "devnet" | "mainnet-beta"): "devnet" | "mainnet" {
  return cluster === "mainnet-beta" ? "mainnet" : "devnet";
}

const launchNebSchema = z.object({
  skip: z.boolean().default(false),
  quoteMint: pubkeySchema,
  token: tokenSchema,
  pool: poolSchema,
});

const renderSchema = z
  .object({
    // Off by default: this is the one step that reaches a real Render
    // account and can redeploy production services. Requires RENDER_API_KEY
    // in the environment (never read from this file) whenever it's not skipped.
    skip: z.boolean().default(true),
    webServiceId: optionalString,
    indexerServiceId: optionalString,
    treasuryAddress: optionalPubkey,
    turnstileSiteKey: optionalString,
    turnstileSecretKey: optionalString,
    // Whether to call the deploy-trigger endpoint after syncing env vars.
    // Both services autoDeploy on push already (render.yaml); this exists
    // for redeploying without a new commit (e.g. after only changing env vars
    // that don't trigger a build on their own).
    triggerDeploy: z.boolean().default(true),
  })
  .default({});

const seedAppsSchema = z
  .object({
    skip: z.boolean().default(true),
    file: z.string().default("scripts/appData/apps.json"),
    limit: z.number().int().positive().optional(),
  })
  .default({});

export const deployConfigSchema = z.object({
  cluster: z.enum(["devnet", "mainnet-beta"]),
  rpcUrl: z.string().url(),
  // Pays for and signs every on-chain step below (program deploy, Config
  // init, NEB launch, app seeding) and becomes the deployed program's
  // upgrade authority. Needs a real SOL balance on `cluster` before running
  // with dryRun: false.
  deployerKeypairPath: z.string().min(1),
  // Global dry run — every step logs its plan without sending transactions
  // or calling Render. Flip to false only after reviewing that plan.
  dryRun: z.boolean().default(true),

  program: z.object({ skip: z.boolean().default(false) }).default({}),

  config: z
    .object({ skip: z.boolean().default(false), protocolFeeBps: z.number().int().min(0).max(10000).default(250) })
    .default({}),

  // Omit (or set skip: true) to reuse an already-launched NEB mint/pool —
  // set existingVoteTokenMint/existingDlmmPool below in that case so later
  // steps (Config init, Render sync) still know the addresses.
  launchNeb: launchNebSchema.optional(),
  existingVoteTokenMint: optionalPubkey,
  existingDlmmPool: optionalPubkey,

  render: renderSchema,
  seedApps: seedAppsSchema,
});

export type DeployConfig = z.infer<typeof deployConfigSchema>;

export function loadConfig(path: string): DeployConfig {
  const raw = readFileSync(path, "utf-8");
  const errors: import("jsonc-parser").ParseError[] = [];
  const json = parseJsonc(raw, errors, { allowTrailingComma: true });
  if (errors.length > 0) {
    throw new Error(
      `Failed to parse ${path} as JSONC: ${errors.map((e) => `offset ${e.offset}: error code ${e.error}`).join("; ")}`,
    );
  }
  const parsed = deployConfigSchema.parse(json);
  return { ...parsed, deployerKeypairPath: expandHome(parsed.deployerKeypairPath) };
}
