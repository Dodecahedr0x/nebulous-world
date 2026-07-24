"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useWallet } from "@solana/wallet-adapter-react";
import { BN } from "@anchor-lang/core";
import { useAuth } from "@/components/providers/AuthProvider";
import { useToast } from "@/components/ui/Toaster";
import { useClaimRewards } from "@/hooks/useClaimRewards";
import { ConnectButton } from "@/components/ConnectButton";
import { isSimulationMode } from "@/lib/config";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { formatToken } from "@/lib/utils";
import { fromRawAmount } from "@/lib/anchorClient";
import { apiGet } from "@/lib/txClient";
import type { AppAccountData, ClaimItem, PositionData } from "@/lib/indexerClient";
import { settlePendingRaw } from "@/lib/rewards";

interface VotePositionDTO {
  appId: string;
  appSlug: string;
  appName: string;
  amount: number;
}

interface StakePositionDTO {
  appTagId: string;
  appId: string;
  appSlug: string;
  appName: string;
  tagSlug: string;
  tagName: string;
  amount: number;
}

interface ClaimRow {
  key: string;
  kind: "vote" | "tag";
  appId: string;
  appSlug: string;
  appName: string;
  tagSlug?: string;
  tagName?: string;
  pending: number | null; // null while loading or unavailable
}

/**
 * "Your rewards" — every app/tag whose vote or tag stake has actually
 * accrued a claimable NEB reward right now, nothing else. This used to also
 * be the full stake-management surface (search, bulk select, unstake) —
 * that workspace moved to the Profile page's "Your stakes" list (see
 * components/profile/MyStakes.tsx); this stays a short, simple claim list
 * so the Rewards page doesn't repeat it.
 */
export function ClaimRewards() {
  const { user } = useAuth();
  const wallet = useWallet();
  const toast = useToast();
  const { claimVoteReward, claimTagReward, claimAllRewards } = useClaimRewards();

  const [rows, setRows] = useState<ClaimRow[] | null>(null);
  const [claimingKey, setClaimingKey] = useState<string | null>(null);
  const [claimingAll, setClaimingAll] = useState(false);

  useEffect(() => {
    if (!user || isSimulationMode() || !wallet.publicKey) {
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
      const owner = wallet.publicKey!.toBase58();

      const base: ClaimRow[] = [
        ...votes.map((v) => ({
          key: `vote:${v.appId}`,
          kind: "vote" as const,
          appId: v.appId,
          appSlug: v.appSlug,
          appName: v.appName,
          pending: null,
        })),
        ...stakes.map((s) => ({
          key: `tag:${s.appTagId}`,
          kind: "tag" as const,
          appId: s.appId,
          appSlug: s.appSlug,
          appName: s.appName,
          tagSlug: s.tagSlug,
          tagName: s.tagName,
          pending: null,
        })),
      ];

      const withPending = await Promise.all(
        base.map(async (row) => {
          try {
            const { app } = await apiGet<{ app: AppAccountData | null }>(
              `/api/accounts/app/${encodeURIComponent(row.appId)}`,
            );
            if (!app) return row;
            if (row.kind === "vote") {
              const { position } = await apiGet<{ position: PositionData | null }>(
                `/api/accounts/vote-position/${encodeURIComponent(row.appId)}?owner=${owner}`,
              );
              if (!position) return row;
              const pending = settlePendingRaw(
                new BN(position.amount),
                new BN(position.rewardDebt),
                new BN(app.voteAccRewardPerShare),
              );
              return { ...row, pending: fromRawAmount(pending) };
            }
            const { position } = await apiGet<{ position: PositionData | null }>(
              `/api/accounts/stake-position/${encodeURIComponent(row.appId)}/${encodeURIComponent(row.tagSlug!)}?owner=${owner}`,
            );
            if (!position) return row;
            const pending = settlePendingRaw(
              new BN(position.amount),
              new BN(position.rewardDebt),
              new BN(app.tagsAccRewardPerShare),
            );
            return { ...row, pending: fromRawAmount(pending) };
          } catch {
            // Position/app not found on-chain yet — leave pending unknown
            // rather than erroring the whole list; it's filtered out below
            // either way (only a known-positive pending shows up).
            return row;
          }
        }),
      );
      if (!cancelled) setRows(withPending.filter((r) => r.pending != null && r.pending > 0));
    }

    load().catch(() => setRows([]));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user, wallet.publicKey]);

  async function claim(row: ClaimRow) {
    setClaimingKey(row.key);
    try {
      const { txSig, simulated } =
        row.kind === "vote"
          ? await claimVoteReward(row.appId)
          : await claimTagReward(row.appId, row.tagSlug!);

      toast.success(
        simulated
          ? "Claimed (simulated) — running without a live deployment"
          : `Claimed your ${row.appName} reward`,
        txSig ? { txSig } : undefined,
      );
      setRows((prev) => prev?.filter((r) => r.key !== row.key) ?? null);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Claim failed");
    } finally {
      setClaimingKey(null);
    }
  }

  const claimableRows = rows?.filter((r) => !!r.pending) ?? [];
  const totalClaimable = claimableRows.reduce((sum, r) => sum + (r.pending ?? 0), 0);

  async function claimAll() {
    if (claimableRows.length === 0) return;
    setClaimingAll(true);
    try {
      const claims: ClaimItem[] = claimableRows.map((r) =>
        r.kind === "vote"
          ? { kind: "vote", appId: r.appId }
          : { kind: "tag", appId: r.appId, tagSlug: r.tagSlug! },
      );
      const { txSigs, simulated } = await claimAllRewards(claims);
      const plural = claimableRows.length === 1 ? "" : "s";
      toast.success(
        simulated
          ? "Claimed all (simulated) — running without a live deployment"
          : `Claimed ${formatToken(totalClaimable, TOKEN_SYMBOL)} across ${claimableRows.length} position${plural} in ${txSigs.length} transaction${txSigs.length === 1 ? "" : "s"}`,
      );
      setRows((prev) =>
        prev?.map((r) => (claimableRows.some((c) => c.key === r.key) ? { ...r, pending: 0 } : r)) ?? null,
      );
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Claim all failed");
    } finally {
      setClaimingAll(false);
    }
  }

  return (
    <section className="card space-y-4 p-6">
      <div>
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">Your rewards</h2>
        <p className="mt-1 text-xs text-slate-steel">
          Every vote and tag stake earns a share of that app&apos;s funded {TOKEN_SYMBOL} reward
          pool. To unstake your principal, see &quot;Your stakes&quot; on your{" "}
          <Link href="/profile" className="font-medium text-cobalt hover:underline">
            profile page
          </Link>
          .
        </p>
      </div>

      {!user ? (
        <div className="space-y-2">
          <p className="text-sm text-slate">Sign in to see what you can claim.</p>
          <ConnectButton />
        </div>
      ) : isSimulationMode() ? (
        <p className="rounded-lg border border-hairline bg-ivory p-3 text-xs text-slate-steel">
          Running in simulation mode — pending rewards only exist once votes/stakes are settled on
          a real deployment.
        </p>
      ) : !wallet.publicKey ? (
        <div className="space-y-2 rounded-lg border border-hairline bg-ivory p-3">
          <p className="text-xs text-slate-steel">Connect your wallet to see what you can claim.</p>
          <ConnectButton />
        </div>
      ) : rows === null ? (
        <p className="text-sm text-slate">Loading your rewards…</p>
      ) : rows.length === 0 ? (
        <p className="text-sm text-slate">Nothing to claim right now — check back later.</p>
      ) : (
        <>
          {claimableRows.length > 1 && (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-hairline bg-mist p-3">
              <div>
                <div className="text-xs text-slate-steel">Claimable now</div>
                <div className="font-mono text-sm font-medium tabular-nums text-ink">
                  {formatToken(totalClaimable, TOKEN_SYMBOL)}
                </div>
              </div>
              <button
                className="btn-primary text-xs"
                disabled={claimingAll}
                onClick={claimAll}
              >
                {claimingAll ? "Claiming…" : `Claim all (${claimableRows.length})`}
              </button>
            </div>
          )}

          <ul className="max-h-[28rem] space-y-1.5 overflow-y-auto pr-1">
            {rows.map((row) => (
              <li
                key={row.key}
                className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-hairline p-2"
              >
                <div className="flex min-w-0 items-center gap-1.5">
                  <Link href={`/app/${row.appSlug}`} className="truncate text-sm font-medium text-ink hover:text-cobalt">
                    {row.appName}
                  </Link>
                  {row.kind === "tag" ? (
                    <Link href={`/tags/${row.tagSlug}`} className="chip chip-active shrink-0 text-[10px]">
                      #{row.tagName}
                    </Link>
                  ) : (
                    <span className="chip shrink-0 text-[10px]">Vote</span>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="font-mono text-xs font-medium tabular-nums text-ink">
                    {formatToken(row.pending!, TOKEN_SYMBOL)}
                  </span>
                  <button
                    className="btn-primary text-xs"
                    disabled={claimingKey === row.key}
                    onClick={() => claim(row)}
                  >
                    {claimingKey === row.key ? "…" : "Claim"}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
