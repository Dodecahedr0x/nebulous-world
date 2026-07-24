"use client";

import Image from "next/image";
import Link from "next/link";
import { useEffect, useState } from "react";
import { useWallet } from "@solana/wallet-adapter-react";
import { BN } from "@anchor-lang/core";
import { useAuth } from "@/components/providers/AuthProvider";
import { useToast } from "@/components/ui/Toaster";
import { useVoteProgram } from "@/hooks/useVoteProgram";
import { useTagStakeProgram } from "@/hooks/useTagStakeProgram";
import { useClaimRewards } from "@/hooks/useClaimRewards";
import { useMountTransition } from "@/hooks/useMountTransition";
import { ConnectButton } from "@/components/ConnectButton";
import { isSimulationMode } from "@/lib/config";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { cn, formatToken } from "@/lib/utils";
import { fromRawAmount } from "@/lib/anchorClient";
import { apiGet, apiPost } from "@/lib/txClient";
import { estimateUnstakeFee } from "@/lib/unstakeFee";
import { settlePendingRaw } from "@/lib/rewards";
import type { AppAccountData, PositionData } from "@/lib/indexerClient";
import { UnstakeFeeNotice } from "@/components/UnstakeFeeNotice";

interface VotePositionDTO {
  appId: string;
  appSlug: string;
  appName: string;
  appIconUrl: string | null;
  amount: number;
}

interface StakePositionDTO {
  appTagId: string;
  appId: string;
  appSlug: string;
  appName: string;
  appIconUrl: string | null;
  tagSlug: string;
  tagName: string;
  amount: number;
}

interface StakeRow {
  key: string;
  kind: "vote" | "tag";
  appId: string;
  appSlug: string;
  appName: string;
  appIconUrl: string | null;
  appTagId?: string;
  tagSlug?: string;
  tagName?: string;
  stakedAmount: number;
  /** On-chain position's `stakedAt` checkpoint — drives the early-unstake
      fee notice below. Only known once fetched (real deployment + connected
      wallet); null otherwise, which just hides the notice. */
  stakedAt: number | null;
  /** Pending NEB reward, settled live from the on-chain position + this
      app's reward accumulator — same source ClaimRewards uses. Null while
      loading or unavailable (simulation mode, no wallet). */
  pending: number | null;
}

interface AppGroup {
  appId: string;
  appSlug: string;
  appName: string;
  appIconUrl: string | null;
  voteRow?: StakeRow;
  tagRows: StakeRow[];
}

/**
 * "Your stakes" — every app vote and tag stake the signed-in user currently
 * has open, grouped by app, in one compact, scrollable list so it can sit on
 * the Profile page without pushing everything else below the fold. This is
 * the full stake-management surface (unstake — full or partial — and claim);
 * the Rewards page's ClaimRewards is now just a short "what's ready to
 * claim" list that links back here for anything requiring unstaking. Same
 * `/api/rewards/positions` data source as ClaimRewards, plus the same
 * pending-reward on-chain fetch (so Claim can live here too).
 */
export function MyStakes() {
  const { user } = useAuth();
  const wallet = useWallet();
  const toast = useToast();
  const { withdrawVote } = useVoteProgram();
  const { withdrawTagStake } = useTagStakeProgram();
  const { claimVoteReward, claimTagReward } = useClaimRewards();

  const [rows, setRows] = useState<StakeRow[] | null>(null);
  const [claimingKey, setClaimingKey] = useState<string | null>(null);
  const [unstakingKey, setUnstakingKey] = useState<string | null>(null);
  // Which row's unstake amount panel is open, and the amount currently
  // entered there — partial unstaking means this can be less than the
  // row's full stakedAmount (see indexer/src/handlers/votes.rs's/
  // stakes.rs's withdraw_partial).
  const [openUnstakeKey, setOpenUnstakeKey] = useState<string | null>(null);
  const { rendered: unstakeRendered, visible: unstakeVisible } = useMountTransition(openUnstakeKey, 200);
  const [unstakeAmount, setUnstakeAmount] = useState(0);

  useEffect(() => {
    if (!user) {
      setRows(null);
      return;
    }

    let cancelled = false;

    async function load() {
      const res = await fetch("/api/rewards/positions");
      const json = await res.json();
      if (cancelled || !json.ok) return;

      const votes: VotePositionDTO[] = json.data.votes;
      const stakes: StakePositionDTO[] = json.data.stakes;

      const base: StakeRow[] = [
        ...votes.map((v) => ({
          key: `vote:${v.appId}`,
          kind: "vote" as const,
          appId: v.appId,
          appSlug: v.appSlug,
          appName: v.appName,
          appIconUrl: v.appIconUrl,
          stakedAmount: v.amount,
          stakedAt: null,
          pending: null,
        })),
        ...stakes.map((s) => ({
          key: `tag:${s.appTagId}`,
          kind: "tag" as const,
          appId: s.appId,
          appSlug: s.appSlug,
          appName: s.appName,
          appIconUrl: s.appIconUrl,
          appTagId: s.appTagId,
          tagSlug: s.tagSlug,
          tagName: s.tagName,
          stakedAmount: s.amount,
          stakedAt: null,
          pending: null,
        })),
      ];
      setRows(base);

      if (isSimulationMode() || !wallet.publicKey) return;

      const owner = wallet.publicKey.toBase58();
      const withDetail = await Promise.all(
        base.map(async (row) => {
          try {
            const { app } = await apiGet<{ app: AppAccountData | null }>(`/api/accounts/app/${row.appId}`);
            const path =
              row.kind === "vote"
                ? `/api/accounts/vote-position/${encodeURIComponent(row.appId)}?owner=${owner}`
                : `/api/accounts/stake-position/${encodeURIComponent(row.appId)}/${encodeURIComponent(row.tagSlug!)}?owner=${owner}`;
            const { position } = await apiGet<{ position: PositionData | null }>(path);
            if (!position) return row;
            const pending = app
              ? fromRawAmount(
                  settlePendingRaw(
                    new BN(position.amount),
                    new BN(position.rewardDebt),
                    new BN(row.kind === "vote" ? app.voteAccRewardPerShare : app.tagsAccRewardPerShare),
                  ),
                )
              : null;
            return { ...row, stakedAt: position.stakedAt, pending };
          } catch {
            // Position not found on-chain yet (e.g. simulation-seeded data
            // with no matching real account) — leave fee/pending hidden
            // rather than erroring the whole list.
            return row;
          }
        }),
      );
      if (!cancelled) setRows(withDetail);
    }

    load().catch(() => setRows([]));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user, wallet.publicKey]);

  const walletReady = isSimulationMode() || !!wallet.publicKey;

  function openUnstake(row: StakeRow) {
    setUnstakeAmount(row.stakedAmount);
    setOpenUnstakeKey(openUnstakeKey === row.key ? null : row.key);
  }

  async function unstake(row: StakeRow) {
    if (unstakeAmount <= 0 || unstakeAmount > row.stakedAmount) return;
    setUnstakingKey(row.key);
    try {
      const { txSig, simulated } =
        row.kind === "vote"
          ? await withdrawVote(row.appId, unstakeAmount)
          : await withdrawTagStake(row.appId, row.tagSlug!, unstakeAmount);

      await apiPost(
        row.kind === "vote" ? "/api/vote/withdraw-partial" : "/api/stake/withdraw-partial",
        row.kind === "vote"
          ? { appId: row.appId, amount: unstakeAmount }
          : { appTagId: row.appTagId, amount: unstakeAmount },
      );

      const full = unstakeAmount >= row.stakedAmount;
      toast.success(
        simulated ? "Unstaked (simulated)" : "Unstaked — tokens returned",
        txSig ? { txSig } : undefined,
      );
      setRows((prev) =>
        prev
          ?.map((r) =>
            r.key === row.key
              ? full
                ? null
                // withdrawVote/withdrawTagStake settle (and pay out) any
                // pending reward unconditionally before moving principal —
                // see withdraw_vote.rs/withdraw_tag_stake.rs — so a partial
                // unstake already zeroed it on-chain too, not just the
                // withdrawn amount.
                : { ...r, stakedAmount: r.stakedAmount - unstakeAmount, pending: 0 }
              : r,
          )
          .filter((r): r is StakeRow => r !== null) ?? null,
      );
      setOpenUnstakeKey(null);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Unstake failed");
    } finally {
      setUnstakingKey(null);
    }
  }

  async function claim(row: StakeRow) {
    setClaimingKey(row.key);
    try {
      const { txSig, simulated } =
        row.kind === "vote"
          ? await claimVoteReward(row.appId)
          : await claimTagReward(row.appId, row.tagSlug!);
      toast.success(
        simulated ? "Claimed (simulated) — running without a live deployment" : `Claimed your ${row.appName} reward`,
        txSig ? { txSig } : undefined,
      );
      setRows((prev) => prev?.map((r) => (r.key === row.key ? { ...r, pending: 0 } : r)) ?? null);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Claim failed");
    } finally {
      setClaimingKey(null);
    }
  }

  const groups: AppGroup[] = [];
  const groupByApp = new Map<string, AppGroup>();
  for (const row of rows ?? []) {
    let group = groupByApp.get(row.appId);
    if (!group) {
      group = {
        appId: row.appId,
        appSlug: row.appSlug,
        appName: row.appName,
        appIconUrl: row.appIconUrl,
        tagRows: [],
      };
      groupByApp.set(row.appId, group);
      groups.push(group);
    }
    if (row.kind === "vote") group.voteRow = row;
    else group.tagRows.push(row);
  }

  function renderRow(row: StakeRow) {
    const fee = row.stakedAt != null ? estimateUnstakeFee(row.stakedAmount, row.stakedAt) : null;
    const busy = unstakingKey === row.key || claimingKey === row.key;
    return (
      <li key={row.key}>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-1.5">
            {row.kind === "tag" ? (
              <Link href={`/tags/${row.tagSlug}`} className="chip chip-active shrink-0 text-[10px]">
                #{row.tagName}
              </Link>
            ) : (
              <span className="chip shrink-0 text-[10px]">Vote</span>
            )}
            <span className="whitespace-nowrap font-mono text-xs tabular-nums text-slate-steel">
              {formatToken(row.stakedAmount, TOKEN_SYMBOL)}
            </span>
          </div>
          {/* flex-wrap here (not just on the row above) — the fee notice's
              text can be too long to sit next to the Unstake button even
              within this column's own width once it wraps below the left
              side on a narrow card, so it needs its own fallback to drop to
              a second line rather than overflow. */}
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            {fee && <UnstakeFeeNotice feeBps={fee.feeBps} />}
            <button
              className="btn-secondary px-2.5 py-1 text-[11px]"
              disabled={busy || !walletReady}
              onClick={() => openUnstake(row)}
            >
              {openUnstakeKey === row.key ? "Cancel" : "Unstake"}
            </button>
            {!isSimulationMode() && (
              <button
                className="btn-primary px-2.5 py-1 text-[11px]"
                disabled={busy || !wallet.publicKey || !row.pending}
                onClick={() => claim(row)}
              >
                {claimingKey === row.key ? "…" : row.pending ? `Claim ${formatToken(row.pending, "")}` : "Claim"}
              </button>
            )}
          </div>
        </div>
        {unstakeRendered === row.key && (
          <div
            className={cn(
              "mt-2 flex items-center gap-2 transition-opacity duration-200 motion-safe:transition-[opacity,transform]",
              unstakeVisible
                ? "opacity-100 motion-safe:translate-y-0"
                : "opacity-0 motion-safe:-translate-y-1",
            )}
          >
            <input
              type="number"
              min={0}
              max={row.stakedAmount}
              step="any"
              className="input min-w-0 py-1 text-xs"
              value={unstakeAmount}
              onChange={(e) => setUnstakeAmount(Math.max(0, Number(e.target.value)))}
              aria-label="Unstake amount"
            />
            <button
              className="btn-primary shrink-0 px-2.5 py-1 text-[11px]"
              disabled={unstakingKey === row.key || unstakeAmount <= 0 || unstakeAmount > row.stakedAmount}
              onClick={() => unstake(row)}
            >
              {unstakingKey === row.key ? "…" : "Confirm"}
            </button>
          </div>
        )}
      </li>
    );
  }

  return (
    <section className="card space-y-3 p-4">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">Your stakes</h2>
        <Link href="/rewards" className="text-xs font-medium text-cobalt hover:underline">
          Claim rewards →
        </Link>
      </div>

      {!user ? (
        <div className="space-y-2">
          <p className="text-sm text-slate">Sign in to see and manage your stakes.</p>
          <ConnectButton />
        </div>
      ) : rows === null ? (
        <p className="text-sm text-slate">Loading your stakes…</p>
      ) : groups.length === 0 ? (
        <p className="text-sm text-slate">
          You haven&apos;t voted or staked on any apps yet.{" "}
          <Link href="/" className="font-medium text-cobalt hover:underline">
            Discover an app
          </Link>{" "}
          to get started.
        </p>
      ) : (
        // Capped height + scroll is the whole point: a wallet with a dozen+
        // positions must not push the rest of the Profile page below the
        // fold — see this component's own doc comment.
        <div className="max-h-96 space-y-3 overflow-y-auto pr-1">
          {groups.map((group) => (
            <div key={group.appId} className="rounded-lg border border-hairline p-2">
              <Link
                href={`/app/${group.appSlug}`}
                className="flex items-center gap-2 text-sm font-medium text-ink hover:text-cobalt"
              >
                <span className="relative h-5 w-5 shrink-0 overflow-hidden rounded-full bg-mist">
                  {group.appIconUrl ? (
                    <Image src={group.appIconUrl} alt="" fill sizes="20px" className="object-cover" />
                  ) : (
                    <span className="grid h-full w-full place-items-center text-[10px] font-bold text-violet">
                      {group.appName.charAt(0).toUpperCase() || "?"}
                    </span>
                  )}
                </span>
                {group.appName}
              </Link>
              <ul className="mt-1.5 space-y-2 border-l border-hairline pl-2">
                {group.voteRow && renderRow(group.voteRow)}
                {group.tagRows.map(renderRow)}
              </ul>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
