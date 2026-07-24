// Single end-to-end production/staging deploy script: Anchor program
// deploy, Config initialization, NEB token/DLMM pool launch, Render env var
// sync + redeploy, and optional app seeding — the manual steps documented
// across this README's "Populating apps"/"NEB token launch" sections plus
// the Render dashboard entry render.yaml's `sync: false` vars require,
// driven by one JSONC config instead. See deploy.config.example.jsonc.
//
// Defaults to dryRun: true everywhere — every step logs its plan without
// sending transactions, running anchor/npm subprocesses, or calling
// Render's API. Set "dryRun": false only after reviewing that plan.
//
// Usage:
//   tsx scripts/deploy/index.ts [--config=./deploy.config.json]

import { readFileSync } from "fs";
import { execFileSync } from "child_process";
import path from "path";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { loadConfig, toAnchorCluster, type DeployConfig } from "./config";
import { setEnvVar, triggerDeploy } from "./render";
import { ensureConfigInitialized } from "../ensureConfigInitialized";
import { createTokenWithMetadata } from "../launch-neb/token";
import { createLaunchPool } from "../launch-neb/pool";
import type { LaunchConfig } from "../launch-neb/config";

const APP_DIR = path.resolve(__dirname, "..", "..");
const REPO_ROOT = path.resolve(__dirname, "..", "..", "..");
const ANCHOR_TOML_PATH = path.join(REPO_ROOT, "Anchor.toml");

const MONIKER_DEFAULT_RPC: Record<string, string> = {
  devnet: "https://api.devnet.solana.com",
  mainnet: "https://api.mainnet-beta.solana.com",
};

function parseArgs(argv: string[]): { configPath: string } {
  const configArg = argv.find((a) => a.startsWith("--config="));
  return { configPath: configArg ? configArg.slice("--config=".length) : "./deploy.config.json" };
}

/** Reads the program id already declared for `anchorCluster` in Anchor.toml. Never writes to Anchor.toml — adding a `[programs.*]` section is a one-time, low-frequency, high-stakes edit left to a human. */
function readDeclaredProgramId(anchorCluster: "devnet" | "mainnet"): string {
  const text = readFileSync(ANCHOR_TOML_PATH, "utf-8");
  const section = text.match(new RegExp(`\\[programs\\.${anchorCluster}\\]([\\s\\S]*?)(?=\\n\\[|$)`));
  if (!section) {
    throw new Error(
      `Anchor.toml has no [programs.${anchorCluster}] section. Add one with the program's declared id ` +
        `before deploying to this cluster — this script deliberately won't add it for you.`,
    );
  }
  const idMatch = section[1].match(/nebulous_world\s*=\s*"([^"]+)"/);
  if (!idMatch) throw new Error(`[programs.${anchorCluster}] in Anchor.toml has no nebulous_world entry`);
  return idMatch[1]!;
}

function loadKeypair(filePath: string): Keypair {
  return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(filePath, "utf-8"))));
}

function runProgramDeploy(cfg: DeployConfig, anchorCluster: "devnet" | "mainnet") {
  console.log("\n== Program deploy ==");
  if (cfg.program.skip) {
    console.log("  skipped (program.skip: true)");
    return;
  }
  const rpcNote =
    MONIKER_DEFAULT_RPC[anchorCluster] !== cfg.rpcUrl
      ? ` (note: anchor uses its own default ${anchorCluster} RPC unless ANCHOR_PROVIDER_URL is set — passing it through as ${cfg.rpcUrl})`
      : "";
  console.log(`  anchor build && anchor deploy --provider.cluster ${anchorCluster}${rpcNote}`);
  if (cfg.dryRun) {
    console.log("  [dry run] not running anchor");
    return;
  }
  const env = { ...process.env, ANCHOR_WALLET: cfg.deployerKeypairPath, ANCHOR_PROVIDER_URL: cfg.rpcUrl };
  execFileSync("anchor", ["build"], { cwd: REPO_ROOT, stdio: "inherit", env });
  execFileSync("anchor", ["deploy", "--provider.cluster", anchorCluster], { cwd: REPO_ROOT, stdio: "inherit", env });
}

async function runConfigInit(
  cfg: DeployConfig,
  connection: Connection,
  programId: PublicKey,
  voteMint: PublicKey | null,
  deployer: Keypair,
) {
  console.log("\n== Config initialization ==");
  if (cfg.config.skip) {
    console.log("  skipped (config.skip: true)");
    return;
  }
  if (!voteMint) {
    throw new Error(
      "config.skip is false but no vote token mint is available — either don't skip launchNeb, " +
        "or set existingVoteTokenMint in the deploy config.",
    );
  }
  console.log(`  programId=${programId.toBase58()} voteMint=${voteMint.toBase58()} feeBps=${cfg.config.protocolFeeBps}`);
  if (cfg.dryRun) {
    console.log("  [dry run] not sending the initialize transaction");
    return;
  }
  await ensureConfigInitialized(connection, programId, voteMint, deployer, cfg.config.protocolFeeBps);
}

async function runLaunchNeb(
  cfg: DeployConfig,
  connection: Connection,
  deployer: Keypair,
): Promise<{ mint: PublicKey; pool: PublicKey | null } | null> {
  console.log("\n== NEB token/DLMM launch ==");
  if (!cfg.launchNeb || cfg.launchNeb.skip) {
    console.log("  skipped (launchNeb.skip: true or launchNeb omitted)");
    if (cfg.existingVoteTokenMint) {
      console.log(`  using existingVoteTokenMint=${cfg.existingVoteTokenMint}`);
      return {
        mint: new PublicKey(cfg.existingVoteTokenMint),
        pool: cfg.existingDlmmPool ? new PublicKey(cfg.existingDlmmPool) : null,
      };
    }
    return null;
  }
  const launchConfig: LaunchConfig = {
    rpcUrl: cfg.rpcUrl,
    cluster: cfg.cluster,
    keypairFilePath: cfg.deployerKeypairPath,
    quoteMint: cfg.launchNeb.quoteMint,
    dryRun: cfg.dryRun,
    token: cfg.launchNeb.token,
    pool: cfg.launchNeb.pool,
  };
  const { mint, totalSupplyRaw } = await createTokenWithMetadata(connection, deployer, launchConfig);
  const pool = await createLaunchPool(connection, deployer, mint, totalSupplyRaw, launchConfig);
  return { mint, pool: pool?.poolAddress ?? null };
}

async function runRenderSync(
  cfg: DeployConfig,
  programId: PublicKey,
  voteMint: PublicKey | null,
  dlmmPool: PublicKey | null,
) {
  console.log("\n== Render env var sync ==");
  if (cfg.render.skip) {
    console.log("  skipped (render.skip: true — the default). Set these manually in the Render dashboard instead:");
    if (voteMint) console.log(`    nebulous-world, nebulous-world-indexer: NEXT_PUBLIC_VOTE_TOKEN_MINT=${voteMint.toBase58()}`);
    if (dlmmPool) console.log(`    nebulous-world-indexer: NEXT_PUBLIC_NEB_DLMM_POOL=${dlmmPool.toBase58()}`);
    if (cfg.render.treasuryAddress) console.log(`    nebulous-world: NEXT_PUBLIC_TREASURY_ADDRESS=${cfg.render.treasuryAddress}`);
    console.log(`    nebulous-world: NEXT_PUBLIC_NEBULOUS_WORLD_PROGRAM_ID=${programId.toBase58()}`);
    return;
  }
  const { webServiceId, indexerServiceId } = cfg.render;
  if (!webServiceId || !indexerServiceId) {
    throw new Error("render.skip is false but render.webServiceId/indexerServiceId are not set");
  }

  if (voteMint) {
    await setEnvVar(webServiceId, "NEXT_PUBLIC_VOTE_TOKEN_MINT", voteMint.toBase58(), cfg.dryRun);
    await setEnvVar(indexerServiceId, "NEXT_PUBLIC_VOTE_TOKEN_MINT", voteMint.toBase58(), cfg.dryRun);
  }
  if (dlmmPool) {
    await setEnvVar(indexerServiceId, "NEXT_PUBLIC_NEB_DLMM_POOL", dlmmPool.toBase58(), cfg.dryRun);
  }
  if (cfg.render.treasuryAddress) {
    await setEnvVar(webServiceId, "NEXT_PUBLIC_TREASURY_ADDRESS", cfg.render.treasuryAddress, cfg.dryRun);
  }
  if (cfg.render.turnstileSiteKey) {
    await setEnvVar(webServiceId, "NEXT_PUBLIC_TURNSTILE_SITE_KEY", cfg.render.turnstileSiteKey, cfg.dryRun);
  }
  if (cfg.render.turnstileSecretKey) {
    await setEnvVar(webServiceId, "TURNSTILE_SECRET_KEY", cfg.render.turnstileSecretKey, cfg.dryRun);
  }

  if (cfg.render.triggerDeploy) {
    await triggerDeploy(webServiceId, cfg.dryRun);
    await triggerDeploy(indexerServiceId, cfg.dryRun);
  }
}

function runSeedApps(cfg: DeployConfig, programId: PublicKey) {
  console.log("\n== App seeding ==");
  if (cfg.seedApps.skip) {
    console.log("  skipped (seedApps.skip: true — the default)");
    return;
  }
  const args = ["run", "apps:create-onchain", "--", `--file=${cfg.seedApps.file}`];
  if (cfg.seedApps.limit) args.push(`--limit=${cfg.seedApps.limit}`);
  if (cfg.dryRun) args.push("--dry-run");
  console.log(`  npm ${args.join(" ")}`);
  execFileSync("npm", args, {
    cwd: APP_DIR,
    stdio: "inherit",
    env: {
      ...process.env,
      DEPLOYER_KEYPAIR_PATH: cfg.deployerKeypairPath,
      NEXT_PUBLIC_SOLANA_RPC: cfg.rpcUrl,
      NEXT_PUBLIC_NEBULOUS_WORLD_PROGRAM_ID: programId.toBase58(),
    },
  });
}

async function main() {
  const { configPath } = parseArgs(process.argv.slice(2));
  console.log(`Loading config from ${configPath}`);
  const cfg = loadConfig(configPath);
  const anchorCluster = toAnchorCluster(cfg.cluster);
  const programId = new PublicKey(readDeclaredProgramId(anchorCluster));
  const deployer = loadKeypair(cfg.deployerKeypairPath);
  const connection = new Connection(cfg.rpcUrl, "confirmed");

  console.log(`Cluster: ${cfg.cluster} (Anchor moniker: ${anchorCluster})`);
  console.log(`Deployer: ${deployer.publicKey.toBase58()}`);
  console.log(`Program id: ${programId.toBase58()}`);
  console.log(`Dry run: ${cfg.dryRun}${cfg.dryRun ? ` (set "dryRun": false in ${configPath} to actually run this)` : ""}`);

  runProgramDeploy(cfg, anchorCluster);
  const launched = await runLaunchNeb(cfg, connection, deployer);
  await runConfigInit(cfg, connection, programId, launched?.mint ?? null, deployer);
  await runRenderSync(cfg, programId, launched?.mint ?? null, launched?.pool ?? null);
  runSeedApps(cfg, programId);

  console.log("\n== Done ==");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
