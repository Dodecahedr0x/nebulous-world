"use client";

import { useCallback } from "react";
import { Transaction } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import type { ProgramTxResult } from "@/lib/anchorClient";
import { runProgramTx, submitSigned } from "@/lib/txClient";
import { isSimulationMode } from "@/lib/config";
import { buildClaimAllRewardsTx, type ClaimItem } from "@/lib/indexerClient";

/**
 * Claims a pending vote-pool or tag-pool reward without withdrawing any
 * staked principal — the frontend counterpart to the Anchor program's
 * `claim_vote_reward`/`claim_tag_reward` instructions. Mirrors
 * useVoteProgram/useTagStakeProgram's shape: the indexer builds the
 * unsigned transaction (see app/api/tx/claim-vote-reward,
 * app/api/tx/claim-tag-reward), the wallet signs it locally.
 */
export function useClaimRewards() {
  const wallet = useWallet();

  const claimVoteReward = useCallback(
    async (appId: string): Promise<ProgramTxResult> =>
      runProgramTx(wallet, "/api/tx/claim-vote-reward", { appId }),
    [wallet],
  );

  const claimTagReward = useCallback(
    async (appId: string, tagSlug: string): Promise<ProgramTxResult> =>
      runProgramTx(wallet, "/api/tx/claim-tag-reward", { appId, tagSlug }),
    [wallet],
  );

  // Claims every position in `claims` with a single wallet approval: the
  // indexer packs them into the minimum number of transactions that fit
  // Solana's size limit (see api.rs's build_claim_all_rewards), then
  // `signAllTransactions` signs all of them in one popup instead of one per
  // claim. Falls back loudly rather than silently degrading to one-by-one
  // signing if the connected wallet doesn't support batch signing.
  const claimAllRewards = useCallback(
    async (claims: ClaimItem[]): Promise<{ txSigs: string[]; simulated: boolean }> => {
      if (isSimulationMode()) return { txSigs: [], simulated: true };
      if (!wallet.publicKey) throw new Error("Connect your wallet first");
      if (!wallet.signAllTransactions) {
        throw new Error("Your wallet doesn't support signing multiple transactions at once");
      }

      const { transactions } = await buildClaimAllRewardsTx(claims, wallet.publicKey.toBase58());
      const unsigned = transactions.map((t) => Transaction.from(Buffer.from(t, "base64")));
      const signed = await wallet.signAllTransactions(unsigned);

      // Each packed transaction claims a distinct position (its own PDA), so
      // they have no ordering dependency on one another — submit and confirm
      // them concurrently instead of paying N sequential confirm-waits.
      const txSigs = await Promise.all(signed.map((tx) => submitSigned(tx)));
      return { txSigs, simulated: false };
    },
    [wallet],
  );

  return { claimVoteReward, claimTagReward, claimAllRewards };
}
