"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { BN } from "@anchor-lang/core";
import { useAuth } from "@/components/providers/AuthProvider";
import { useToast } from "@/components/ui/Toaster";
import { useVoteProgram } from "@/hooks/useVoteProgram";
import { useClaimRewards } from "@/hooks/useClaimRewards";
import { isSimulationMode } from "@/lib/config";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { formatToken } from "@/lib/utils";
import { fromRawAmount } from "@/lib/anchorClient";
import { estimateUnstakeFee } from "@/lib/unstakeFee";
import { settlePendingRaw } from "@/lib/rewards";
import { apiGet } from "@/lib/txClient";
import type { AppAccountData, PositionData } from "@/lib/indexerClient";
import { ConnectButton } from "@/components/ConnectButton";
import { UnstakeFeeNotice } from "@/components/UnstakeFeeNotice";

const PRESETS = [10, 50, 100, 500];

/**
 * The vote widget. Commits vote-tokens to an app: settles an on-chain transfer
 * (or simulates it), then records the vote server-side where it feeds ranking.
 */
export function VotePanel({ appId }: { appId: string }) {
  const { user } = useAuth();
  const router = useRouter();
  const toast = useToast();
  const { vote: castVote, withdrawVote } = useVoteProgram();
  const { claimVoteReward } = useClaimRewards();
  const [amount, setAmount] = useState(50);
  const [busy, setBusy] = useState(false);
  const [myVote, setMyVote] = useState<{ id: string; amount: number } | null>(null);
  // Unix seconds, from the on-chain VotePosition's `stakedAt` — drives the
  // unstake-fee estimate shown next to the withdraw button. Fetched
  // separately from `myVote` (a Postgres row) since only the indexed
  // on-chain account carries this field.
  const [stakedAt, setStakedAt] = useState<number | null>(null);
  // Pending NEB reward, settled live from the on-chain position + app
  // accumulator (see lib/rewards.ts) — same claim path as the rewards
  // page's ClaimRewards, surfaced here too so a user doesn't have to leave
  // this app's page to claim what its own vote has earned.
  const [pending, setPending] = useState<number | null>(null);

  useEffect(() => {
    if (!user) {
      setMyVote(null);
      return;
    }
    fetch(`/api/vote?appId=${appId}`)
      .then((res) => res.json())
      .then((json) => setMyVote(json.ok ? json.data.vote : null))
      .catch(() => {});
  }, [appId, user]);

  useEffect(() => {
    if (!user || !myVote) {
      setStakedAt(null);
      setPending(null);
      return;
    }
    let cancelled = false;

    async function load() {
      let position: PositionData | null = null;
      try {
        const res = await apiGet<{ position: PositionData | null }>(
          `/api/accounts/vote-position/${appId}?owner=${user!.wallet}`,
        );
        position = res.position;
      } catch {
        position = null;
      }
      if (cancelled) return;
      setStakedAt(position?.stakedAt ?? null);

      if (!position || isSimulationMode()) {
        setPending(null);
        return;
      }
      try {
        const { app } = await apiGet<{ app: AppAccountData | null }>(`/api/accounts/app/${appId}`);
        if (cancelled) return;
        setPending(
          app
            ? fromRawAmount(
                settlePendingRaw(new BN(position.amount), new BN(position.rewardDebt), new BN(app.voteAccRewardPerShare)),
              )
            : null,
        );
      } catch {
        if (!cancelled) setPending(null);
      }
    }

    load();
    return () => {
      cancelled = true;
    };
  }, [appId, user, myVote]);

  const unstakeFee = myVote && stakedAt !== null ? estimateUnstakeFee(myVote.amount, stakedAt) : null;

  async function claim() {
    setBusy(true);
    try {
      const { txSig, simulated } = await claimVoteReward(appId);
      toast.success(
        simulated ? "Claimed (simulated) — running without a live deployment" : "Claimed your vote reward",
        txSig ? { txSig } : undefined,
      );
      setPending(0);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Claim failed");
    } finally {
      setBusy(false);
    }
  }

  async function vote() {
    if (amount <= 0) return;
    setBusy(true);
    try {
      // 1. Settle the token transfer (real tx in on-chain mode; no-op otherwise).
      const { txSig, simulated } = await castVote(appId, amount);

      // 2. Record the vote.
      const res = await fetch("/api/vote", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ appId, amount, txSig: txSig ?? undefined }),
      });
      const json = await res.json();
      if (!json.ok) throw new Error(json.error || "Vote failed");

      toast.success(
        simulated
          ? `Voted ${amount.toFixed(2)} ${TOKEN_SYMBOL} (simulated)`
          : `Voted ${amount.toFixed(2)} ${TOKEN_SYMBOL} — tx confirmed`,
        txSig ? { txSig } : undefined,
      );
      router.refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Vote failed");
    } finally {
      setBusy(false);
    }
  }

  async function withdraw() {
    if (!myVote) return;
    setBusy(true);
    try {
      const { txSig, simulated } = await withdrawVote(appId, myVote.amount);

      const res = await fetch("/api/vote/withdraw", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ voteId: myVote.id }),
      });
      const json = await res.json();
      if (!json.ok) throw new Error(json.error || "Withdraw failed");

      toast.success(
        simulated
          ? "Vote withdrawn (simulated)"
          : "Vote withdrawn — tokens returned",
        txSig ? { txSig } : undefined,
      );
      setMyVote(null);
      router.refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Withdraw failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card space-y-4 p-6">
      <div>
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">
          Vote for this app
        </h2>
        <p className="mt-1 text-xs text-slate-steel">
          Votes are token-weighted and boost this app&apos;s rank.
          {isSimulationMode() && " Running in simulation mode — no real tokens spent."}
        </p>
      </div>

      {!user ? (
        <div className="space-y-2">
          <p className="text-sm text-slate">Sign in to vote.</p>
          <ConnectButton />
        </div>
      ) : (
        <>
          <div className="flex flex-wrap gap-2">
            {PRESETS.map((p) => (
              <button
                key={p}
                onClick={() => setAmount(p)}
                className={`chip ${amount === p ? "chip-active" : ""}`}
              >
                {p}
              </button>
            ))}
          </div>
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={1}
              className="input"
              value={amount}
              onChange={(e) => setAmount(Math.max(0, Number(e.target.value)))}
              aria-label="Vote amount"
            />
            <span className="text-sm text-slate">{TOKEN_SYMBOL}</span>
          </div>
          <button
            className="btn-primary w-full"
            disabled={busy || amount <= 0}
            onClick={vote}
          >
            {busy ? "Voting…" : `Vote ${amount.toFixed(2)} ${TOKEN_SYMBOL}`}
          </button>
          {myVote && (
            <div className="space-y-1">
              <div className="flex gap-2">
                <button
                  className="btn-secondary flex-1"
                  disabled={busy}
                  onClick={withdraw}
                >
                  {busy ? "…" : `Withdraw ${myVote.amount.toFixed(2)} ${TOKEN_SYMBOL}`}
                </button>
                <button
                  className="btn-primary flex-1"
                  disabled={busy || isSimulationMode() || !pending}
                  onClick={claim}
                >
                  {busy ? "…" : pending ? `Claim ${formatToken(pending, "")} ${TOKEN_SYMBOL}` : "Claim"}
                </button>
              </div>
              {unstakeFee && (
                <div className="flex justify-center">
                  <UnstakeFeeNotice feeBps={unstakeFee.feeBps} />
                </div>
              )}
            </div>
          )}
        </>
      )}
    </section>
  );
}
