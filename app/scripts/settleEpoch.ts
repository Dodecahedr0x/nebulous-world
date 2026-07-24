// Settlement run: pulls real AdSense earnings for the trailing period,
// allocates by traffic share, and funds each app's on-chain reward vaults.
// Requires ADSENSE_CLIENT_ID/ADSENSE_CLIENT_SECRET/ADSENSE_REFRESH_TOKEN (see
// src/lib/adsense.ts's getAdsenseAccessToken for how these mint a fresh
// access token on every run — no manual token minting needed) and a funded
// treasury keypair at TREASURY_KEYPAIR_PATH. Safe to run unattended (e.g.
// from a scheduled job) now that token acquisition is automated.

import { readFileSync } from "fs";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { AnchorProvider, Program, Wallet, BN } from "@anchor-lang/core";
import { getAssociatedTokenAddressSync, getOrCreateAssociatedTokenAccount } from "@solana/spl-token";
import { searchApps, fetchPlatformTraffic } from "../src/lib/indexerClient";
import { fetchAdsenseEarnings, getAdsenseAccessToken } from "../src/lib/adsense";
import { allocateByTrafficShare } from "../src/lib/settlement";
import { config } from "../src/lib/config";
import { appPda, configPda } from "../src/lib/anchorClient";
import idl from "../../target/idl/nebulous_world.json";
import type { NebulousWorld } from "../../target/types/nebulous_world";

const SETTLEMENT_LAG_DAYS = 3; // AdSense finalization lag, per the design doc
// Must match REVENUE_CONFIG.protocolFee in src/lib/revenue.ts. This — not
// on-chain Config.protocol_fee_bps — is the fee's actual enforcement point:
// it's deducted here, off-chain, from gross ad revenue before any of it is
// ever passed to fund_app_rewards. See the doc comment on
// programs/nebulous_world/src/state/config.rs's protocol_fee_bps field.
const PROTOCOL_FEE = 0.1;
const APP_TAG_SPLIT = 0.5; // must match APP_TAG_SPLIT in src/lib/revenue.ts

// A pool is only funded if the on-chain total stake it's about to be funded
// against is at least this fraction of what we computed from the DB moments
// earlier. Guards against the front-running window documented on
// fund_app_rewards (Task 15's architectural note): an attacker who times a
// withdraw_vote/withdraw_tag_stake between our DB read and this script's
// fund_app_rewards call would otherwise inflate their own per-share credit
// for this round at other stakers' expense. Skipping (rather than funding
// against a shrunk denominator) just defers that pool's funding to the next
// epoch — no funds are lost, only delayed.
const MIN_STAKE_FRACTION = 0.95;

async function main() {
  const end = new Date();
  end.setUTCDate(end.getUTCDate() - SETTLEMENT_LAG_DAYS);
  const start = new Date(end);
  start.setUTCDate(start.getUTCDate() - 7);

  const accessToken = await getAdsenseAccessToken();
  const totalEarnings = await fetchAdsenseEarnings({ start, end }, accessToken);
  console.log(`AdSense earnings ${start.toISOString()}–${end.toISOString()}: $${totalEarnings}`);

  // pageSize is set well beyond any realistic catalog size — this script
  // needs every app, not a paginated slice.
  const { apps } = await searchApps({ q: "", tags: [], fuzzy: "", sort: "rank", page: 1, pageSize: 100_000 });
  const trafficByApp = await fetchPlatformTraffic(start, end);
  const traffic = apps.map((app) => ({
    appId: app.id,
    eligibleViews: trafficByApp[app.id] ?? 0,
  }));
  const allocations = allocateByTrafficShare(totalEarnings, traffic);

  const treasuryKeypair = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(process.env.TREASURY_KEYPAIR_PATH!, "utf-8"))),
  );
  const connection = new Connection(config.solana.rpc);
  const provider = new AnchorProvider(connection, new Wallet(treasuryKeypair), {
    commitment: "confirmed",
  });
  const program = new Program<NebulousWorld>(idl as NebulousWorld, provider);
  const cfgPda = configPda(program.programId);
  const mint = new PublicKey(config.solana.voteTokenMint);
  // The single global vault every instruction transfers through (an ATA
  // owned by the `config` PDA) — see programs/nebulous_world/src/state/config.rs.
  const vault = getAssociatedTokenAddressSync(mint, cfgPda, true);
  const treasuryAta = await getOrCreateAssociatedTokenAccount(
    connection,
    treasuryKeypair,
    mint,
    treasuryKeypair.publicKey,
  );
  const decimals = config.solana.voteTokenDecimals;

  for (const alloc of allocations) {
    if (alloc.gross <= 0) continue;
    const app = apps.find((a) => a.id === alloc.appId)!;
    console.log(`Settling ${app.slug}: gross $${alloc.gross}`);

    // App.voteWeight/stakeTotal are cached aggregates kept in sync after
    // every vote/stake mutation (see indexer/src/handlers/engine.rs's
    // refresh_app) — reading them here avoids re-summing the raw Vote/Stake
    // tables the way the original Prisma version did.
    const dbVoteTotal = app.voteWeight;
    const dbTagTotal = app.stakeTotal;
    const hasVoters = app.voteWeight > 0;
    const hasTaggers = app.stakeTotal > 0;

    const fee = alloc.gross * PROTOCOL_FEE;
    const distributable = alloc.gross - fee;
    let voteShare = distributable * APP_TAG_SPLIT;
    let tagShare = distributable - voteShare;
    if (!hasTaggers) {
      voteShare = distributable;
      tagShare = 0;
    } else if (!hasVoters) {
      tagShare = distributable;
      voteShare = 0;
    }

    const app_pda = appPda(program.programId, app.id);
    let appAccount;
    try {
      appAccount = await program.account.appAccount.fetch(app_pda);
    } catch {
      // Apps are listed off-chain immediately on submission (see the design
      // doc's moderation section) but only get an on-chain AppAccount once
      // someone actually votes/stakes on-chain — most apps won't have one
      // yet. Skip rather than aborting the whole settlement run over it.
      console.warn(`  skipping ${app.slug}: no on-chain AppAccount yet (never voted/staked on-chain)`);
      continue;
    }
    const onChainVoteTotal = appAccount.totalVoteStake.toNumber() / 10 ** decimals;
    const onChainTagTotal = appAccount.totalTagStake.toNumber() / 10 ** decimals;

    if (voteShare > 0 && onChainVoteTotal < dbVoteTotal * MIN_STAKE_FRACTION) {
      console.warn(
        `  skipping vote pool: on-chain total_vote_stake (${onChainVoteTotal}) dropped ` +
          `below ${MIN_STAKE_FRACTION * 100}% of the DB total (${dbVoteTotal}) — ` +
          `possible withdrawal race, deferring to next epoch`,
      );
      voteShare = 0;
    }
    if (tagShare > 0 && onChainTagTotal < dbTagTotal * MIN_STAKE_FRACTION) {
      console.warn(
        `  skipping tags pool: on-chain total_tag_stake (${onChainTagTotal}) dropped ` +
          `below ${MIN_STAKE_FRACTION * 100}% of the DB total (${dbTagTotal}) — ` +
          `possible withdrawal race, deferring to next epoch`,
      );
      tagShare = 0;
    }

    if (voteShare > 0) {
      await program.methods
        .fundAppRewards({ vote: {} }, new BN(Math.round(voteShare * 10 ** decimals)))
        .accountsPartial({
          app: app_pda,
          config: cfgPda,
          vault,
          funderTokenAccount: treasuryAta.address,
          authority: treasuryKeypair.publicKey,
        })
        .rpc();
    }
    if (tagShare > 0) {
      await program.methods
        .fundAppRewards({ tags: {} }, new BN(Math.round(tagShare * 10 ** decimals)))
        .accountsPartial({
          app: app_pda,
          config: cfgPda,
          vault,
          funderTokenAccount: treasuryAta.address,
          authority: treasuryKeypair.publicKey,
        })
        .rpc();
    }
  }

  console.log("Settlement complete.");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
