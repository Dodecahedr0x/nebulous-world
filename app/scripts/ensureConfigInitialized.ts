// `Config` (the program's one global singleton — see
// programs/nebulous_world/src/state/config.rs) only ever gets created by a
// single, one-time `initialize` call, signed by the program's upgrade
// authority. Nothing in setup-dev.sh's deploy flow used to do this, so on a
// freshly deployed program `Config` never existed and EVERY vote/stake
// instruction failed with `AccountNotInitialized` — the root cause behind
// "staking gives an error" (voting hit the exact same wall, just less
// visibly since it was usually tried second). setup-dev.sh calls this
// script right after `anchor deploy`; safe to re-run (no-ops if already
// initialized).
//
// Usage: tsx scripts/ensureConfigInitialized.ts

import { readFileSync } from "fs";
import { homedir } from "os";
import { Connection, Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import { AnchorProvider, Program, Wallet } from "@anchor-lang/core";
import { ASSOCIATED_TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { configPda } from "../src/lib/anchorClient";
import { config } from "../src/lib/config";
import idl from "../../target/idl/nebulous_world.json";
import type { NebulousWorld } from "../../target/types/nebulous_world";

const TOKEN_PROGRAM_ID = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
// Owns the BPF Upgradeable Loader's `ProgramData` PDA (seeds = [programId]),
// needed to resolve `initialize`'s `program_data` account.
const BPF_LOADER_UPGRADEABLE_PROGRAM_ID = new PublicKey(
  "BPFLoaderUpgradeab1e11111111111111111111111",
);
// Matches setup-dev.sh's DEV_KEYPAIR — the wallet that deployed the program,
// i.e. its upgrade authority, which is the only signer `initialize` accepts.
const DEV_KEYPAIR_PATH = `${homedir()}/.config/solana/id.json`;

/** Idempotent: does nothing if `Config` already exists on-chain. */
export async function ensureConfigInitialized(
  connection: Connection,
  programId: PublicKey,
  voteMint: PublicKey,
  authority: Keypair,
  protocolFeeBps = 250, // 2.5% — matches the Rust test default
): Promise<void> {
  const cfgPda = configPda(programId);
  if (await connection.getAccountInfo(cfgPda)) {
    console.log("Config already initialized, skipping");
    return;
  }

  const provider = new AnchorProvider(connection, new Wallet(authority), { commitment: "confirmed" });
  const program = new Program<NebulousWorld>(idl as NebulousWorld, provider);
  const [programDataPda] = PublicKey.findProgramAddressSync(
    [programId.toBuffer()],
    BPF_LOADER_UPGRADEABLE_PROGRAM_ID,
  );

  const sig = await program.methods
    .initialize(protocolFeeBps)
    .accountsPartial({
      config: cfgPda,
      authority: authority.publicKey,
      voteMint,
      program: programId,
      programData: programDataPda,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();
  console.log(`Config initialized (tx ${sig.slice(0, 12)}…)`);
}

async function main() {
  if (!config.solana.voteTokenMint || !config.solana.programId) {
    throw new Error(
      "NEXT_PUBLIC_VOTE_TOKEN_MINT / NEXT_PUBLIC_NEBULOUS_WORLD_PROGRAM_ID must be set — " +
        "run scripts/launch-neb (or setup-dev.sh, which runs it) first.",
    );
  }
  const connection = new Connection(config.solana.rpc, "confirmed");
  const devKeypair = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(DEV_KEYPAIR_PATH, "utf-8"))),
  );
  await ensureConfigInitialized(
    connection,
    new PublicKey(config.solana.programId),
    new PublicKey(config.solana.voteTokenMint),
    devKeypair,
  );
}

// Only run as a standalone script when invoked directly (`tsx
// scripts/ensureConfigInitialized.ts`), not when imported elsewhere.
if (require.main === module) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
