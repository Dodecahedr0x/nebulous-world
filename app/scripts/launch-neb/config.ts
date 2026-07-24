// Config schema + loader for the NEB launch script. Parameters are read from
// a JSONC file (comments allowed) rather than env vars/CLI flags — there are
// too many interdependent knobs (token metadata + DLMM curve params) for
// either to stay readable. See launch-neb.config.example.jsonc.

import { readFileSync } from "fs";
import { homedir } from "os";
import { parse as parseJsonc } from "jsonc-parser";
import { z } from "zod";

function expandHome(path: string): string {
  return path.startsWith("~") ? path.replace(/^~/, homedir()) : path;
}

const pubkeySchema = z.string().min(32).max(64);

export const tokenSchema = z.object({
  name: z.string().min(1).max(32),
  symbol: z.string().min(1).max(10),
  decimals: z.number().int().min(0).max(9).default(6),
  totalSupply: z.number().positive(),
  uri: z.string().url(),
  // Whether the mint authority is revoked after minting totalSupply, fixing
  // the supply forever. Defaults on — the whole point of "full supply minted
  // at inception" is that nothing can be minted later.
  revokeMintAuthority: z.boolean().default(true),
  // Whether the on-chain metadata can be updated later (e.g. to fix the URI).
  isMutable: z.boolean().default(true),
});

export const poolSchema = z.object({
  // Price increment between adjacent bins, in basis points. Smaller = finer
  // granularity but fewer bins covered per position.
  binStep: z.number().int().positive(),
  // Trading fee charged on swaps through this pool, in basis points.
  feeBps: z.number().int().min(1).max(1000),
  // Starting price, quote per base (e.g. USDC per NEB).
  initialPrice: z.number().positive(),
  // Multiplier applied to initialPrice for the top of the seeded liquidity
  // range — e.g. 100 spreads the full NEB supply single-sided across every
  // bin from initialPrice up to initialPrice * 100, instead of dumping it
  // all into one fixed-price bin. This lets the pool act as a genuine
  // liquidity provider across a price range as buyers push the price up,
  // rather than selling everything at a single price then going empty.
  maxPriceMultiplier: z.number().min(1).default(100),
  // Which way to round initialPrice to the nearest bin boundary. Used for
  // both the pool's starting active bin and the bottom of the seeded
  // liquidity range — they must agree, or the seed deposit's lowest bin
  // doesn't match the pool's active bin.
  priceRounding: z.enum(["up", "down"]).default("up"),
  // Whether the pool activates by "slot" or by wall-clock "timestamp".
  activationType: z.enum(["slot", "timestamp"]).default("timestamp"),
  // Slot/timestamp the pool starts accepting trades. null = immediately.
  activationPoint: z.number().int().positive().nullable().default(null),
  // Whether the pool creator can pause/resume trading after creation.
  creatorPoolOnOffControl: z.boolean().default(false),
});

export const launchConfigSchema = z.object({
  rpcUrl: z.string().url(),
  // Which cluster's DLMM program to target — must match rpcUrl. Used to look
  // up the correct DLMM program id (it differs per cluster).
  cluster: z.enum(["devnet", "mainnet-beta", "localhost"]).default("devnet"),
  // Path to the keypair that pays for and signs every transaction. It becomes
  // the mint authority (revoked after minting when revokeMintAuthority is
  // set), the token's update authority, and the DLMM pool/position creator.
  keypairFilePath: z.string().min(1),
  // The pool's quote-side mint. Devnet and mainnet USDC are different mints —
  // double-check this against the target cluster before running for real.
  quoteMint: pubkeySchema,
  // When true, prints the planned actions without sending any transactions.
  dryRun: z.boolean().default(true),
  token: tokenSchema,
  pool: poolSchema,
});

export type LaunchConfig = z.infer<typeof launchConfigSchema>;

export function loadConfig(path: string): LaunchConfig {
  const raw = readFileSync(path, "utf-8");
  const errors: import("jsonc-parser").ParseError[] = [];
  const json = parseJsonc(raw, errors, { allowTrailingComma: true });
  if (errors.length > 0) {
    throw new Error(
      `Failed to parse ${path} as JSONC: ${errors.map((e) => `offset ${e.offset}: error code ${e.error}`).join("; ")}`,
    );
  }
  const parsed = launchConfigSchema.parse(json);
  return { ...parsed, keypairFilePath: expandHome(parsed.keypairFilePath) };
}
