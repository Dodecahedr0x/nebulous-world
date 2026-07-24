"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { BN } from "@anchor-lang/core";
import { useAuth } from "@/components/providers/AuthProvider";
import { useToast } from "@/components/ui/Toaster";
import { useTagStakeProgram } from "@/hooks/useTagStakeProgram";
import { useCreateAppProgram } from "@/hooks/useCreateAppProgram";
import { useClaimRewards } from "@/hooks/useClaimRewards";
import { useMountTransition } from "@/hooks/useMountTransition";
import { cn, formatToken, slugify } from "@/lib/utils";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { isSimulationMode } from "@/lib/config";
import { estimateUnstakeFee } from "@/lib/unstakeFee";
import { settlePendingRaw } from "@/lib/rewards";
import { fromRawAmount } from "@/lib/anchorClient";
import { apiGet } from "@/lib/txClient";
import type { AppAccountData, PositionData } from "@/lib/indexerClient";
import { UnstakeFeeNotice } from "@/components/UnstakeFeeNotice";
import type { TagDTO } from "@/lib/types";

/**
 * Tags + staking. Shows each tag with its total stake, lets signed-in users
 * stake tokens behind a tag (settled on-chain in production), and lets anyone
 * suggest a new tag for the app.
 */
export function TagStakePanel({
  appId,
  tags,
}: {
  appId: string;
  tags: TagDTO[];
}) {
  const { user } = useAuth();
  const router = useRouter();
  const toast = useToast();
  const { stakeTag, withdrawTagStake } = useTagStakeProgram();
  const { suggestTag } = useCreateAppProgram();
  const { claimTagReward } = useClaimRewards();

  const [stakingId, setStakingId] = useState<string | null>(null);
  const { rendered: revealRendered, visible: revealVisible } = useMountTransition(stakingId, 200);
  const [withdrawingId, setWithdrawingId] = useState<string | null>(null);
  const { rendered: withdrawRevealRendered, visible: withdrawRevealVisible } = useMountTransition(
    withdrawingId,
    200,
  );
  const [stakeAmount, setStakeAmount] = useState(100);
  const [busy, setBusy] = useState(false);
  const [newTag, setNewTag] = useState("");
  // appTagId -> the current user's active stake on that tag.
  const [myStakes, setMyStakes] = useState<Record<string, { id: string; amount: number }>>({});
  // appTagId -> the on-chain StakePosition's `stakedAt` (Unix seconds) —
  // drives the unstake-fee estimate shown next to each tag's withdraw
  // button. Fetched per-tag since only the indexed on-chain account (not
  // the Postgres `myStakes` row) carries this field.
  const [stakedAtByTag, setStakedAtByTag] = useState<Record<string, number>>({});
  // appTagId -> pending NEB reward, settled live from the on-chain position
  // + this app's tagsAccRewardPerShare (see lib/rewards.ts) — same claim
  // path as the rewards page's ClaimRewards, surfaced here too.
  const [pendingByTag, setPendingByTag] = useState<Record<string, number>>({});

  useEffect(() => {
    if (!user) {
      setMyStakes({});
      return;
    }
    fetch(`/api/stake?appId=${appId}`)
      .then((res) => res.json())
      .then((json) => {
        if (!json.ok) return;
        const byTag: Record<string, { id: string; amount: number }> = {};
        for (const s of json.data.stakes as { id: string; amount: number; appTagId: string }[]) {
          byTag[s.appTagId] = { id: s.id, amount: s.amount };
        }
        setMyStakes(byTag);
      })
      .catch(() => {});
  }, [appId, user]);

  useEffect(() => {
    const tagIds = Object.keys(myStakes);
    if (!user || tagIds.length === 0) {
      setStakedAtByTag({});
      setPendingByTag({});
      return;
    }
    let cancelled = false;

    async function load() {
      // One shared app-level fetch (tagsAccRewardPerShare) rather than one
      // per tag — every tag's pending reward is computed against the same
      // accumulator, just combined with that tag's own position amount/
      // rewardDebt below.
      let app: AppAccountData | null = null;
      if (!isSimulationMode()) {
        try {
          const { app: fetched } = await apiGet<{ app: AppAccountData | null }>(`/api/accounts/app/${appId}`);
          app = fetched;
        } catch {
          app = null;
        }
      }

      const entries = await Promise.all(
        tagIds.map(async (tagId) => {
          const tag = tags.find((t) => t.id === tagId);
          if (!tag) return null;
          try {
            const res = await fetch(
              `/api/accounts/stake-position/${appId}/${tag.slug}?owner=${user!.wallet}`,
            );
            const json = await res.json();
            const position: PositionData | null = json.ok ? json.data.position : null;
            if (!position) return null;
            const pending = app
              ? fromRawAmount(
                  settlePendingRaw(new BN(position.amount), new BN(position.rewardDebt), new BN(app.tagsAccRewardPerShare)),
                )
              : null;
            return [tagId, position.stakedAt as number, pending] as const;
          } catch {
            return null;
          }
        }),
      );
      if (cancelled) return;
      const stakedAtMap: Record<string, number> = {};
      const pendingMap: Record<string, number> = {};
      for (const entry of entries) {
        if (!entry) continue;
        const [tagId, stakedAt, pending] = entry;
        stakedAtMap[tagId] = stakedAt;
        if (pending != null) pendingMap[tagId] = pending;
      }
      setStakedAtByTag(stakedAtMap);
      setPendingByTag(pendingMap);
    }

    load();
    return () => {
      cancelled = true;
    };
  }, [appId, user, myStakes, tags]);

  async function claimTag(tagId: string, tagSlug: string) {
    setBusy(true);
    try {
      const { txSig, simulated } = await claimTagReward(appId, tagSlug);
      toast.success(
        simulated ? "Claimed (simulated) — running without a live deployment" : `Claimed your #${tagSlug} reward`,
        txSig ? { txSig } : undefined,
      );
      setPendingByTag((prev) => ({ ...prev, [tagId]: 0 }));
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Claim failed");
    } finally {
      setBusy(false);
    }
  }

  async function stake(appTagId: string, tagSlug: string) {
    if (stakeAmount <= 0) return;
    setBusy(true);
    try {
      const { txSig, simulated } = await stakeTag(appId, tagSlug, stakeAmount);
      const res = await fetch("/api/stake", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ appTagId, amount: stakeAmount, txSig: txSig ?? undefined }),
      });
      const json = await res.json();
      if (!json.ok) throw new Error(json.error || "Stake failed");
      toast.success(
        simulated
          ? `Staked ${stakeAmount.toFixed(2)} ${TOKEN_SYMBOL} (simulated)`
          : `Staked ${stakeAmount.toFixed(2)} ${TOKEN_SYMBOL}`,
        txSig ? { txSig } : undefined,
      );
      setStakingId(null);
      router.refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Stake failed");
    } finally {
      setBusy(false);
    }
  }

  async function withdraw(appTagId: string, tagSlug: string) {
    const mine = myStakes[appTagId];
    if (!mine) return;
    setBusy(true);
    try {
      const { txSig, simulated } = await withdrawTagStake(appId, tagSlug, mine.amount);

      const res = await fetch("/api/stake/withdraw", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ stakeId: mine.id }),
      });
      const json = await res.json();
      if (!json.ok) throw new Error(json.error || "Withdraw failed");

      toast.success(
        simulated ? "Stake withdrawn (simulated)" : "Stake withdrawn — tokens returned",
        txSig ? { txSig } : undefined,
      );
      setMyStakes((prev) => {
        const next = { ...prev };
        delete next[appTagId];
        return next;
      });
      setWithdrawingId(null);
      router.refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Withdraw failed");
    } finally {
      setBusy(false);
    }
  }

  // Adding a tag is an on-chain `suggest_tag` transaction now (see
  // useCreateAppProgram) — the `Tag`/`AppTag` Postgres rows only show up
  // once the indexer observes it confirmed, same as app creation. There's
  // no synchronous DB write to await here, so `router.refresh()` may not
  // show the new tag immediately; the toast says so rather than implying
  // it's already visible.
  async function suggest() {
    const tagSlug = slugify(newTag);
    if (!tagSlug) return;
    setBusy(true);
    try {
      const txSig = await suggestTag(appId, tagSlug);
      toast.success(`Added #${tagSlug} — indexing…`, { txSig });
      setNewTag("");
      router.refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Could not add tag");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card space-y-4 p-6">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">
          Tags &amp; staking
        </h2>
        <span className="text-xs text-slate-steel">
          Stake backs a tag &amp; earns ad revenue
        </span>
      </div>

      {tags.length === 0 ? (
        <p className="text-sm text-slate-steel">
          No tags yet. Suggest one below.
        </p>
      ) : (
        <ul className="space-y-2">
          {tags.map((t) => (
            <li key={t.id} className="space-y-2 rounded-lg border border-hairline p-3">
              <div>
                <Link href={`/tags/${t.slug}`} className="font-medium text-ink hover:text-cobalt">
                  #{t.name}
                </Link>
                <span className="ml-2 text-xs text-slate-steel">
                  {formatToken(t.stakeTotal, TOKEN_SYMBOL)} staked
                </span>
              </div>
              {/* This panel sits in the app page's sidebar column — narrow
                  enough that three buttons (Withdraw, Claim, Stake) never
                  fit on one line next to the tag info above, so this always
                  stacks info above a button grid rather than trying to keep
                  them side by side and wrapping unpredictably when there
                  isn't room (a plain flex-wrap group used to drop to its own
                  line under the info text, then wrap AGAIN inside itself,
                  leaving whichever button didn't fit stranded alone). A
                  2-column grid gives a controlled result at any count
                  instead: pairs stack cleanly, and a leftover odd button
                  (only Stake, or Withdraw+Claim+Stake) spans the full row
                  rather than sitting alone in one half. */}
              {(() => {
                const buttonCount =
                  (user && myStakes[t.id] ? 1 : 0) +
                  (user && myStakes[t.id] && !isSimulationMode() ? 1 : 0) +
                  (user ? 1 : 0);
                return (
                  <div className={cn("grid grid-cols-2 gap-2", buttonCount % 2 === 1 && "[&>:last-child]:col-span-2")}>
                    {user && myStakes[t.id] && (
                      <button
                        className="btn-secondary text-xs"
                        onClick={() => {
                          setStakingId(null);
                          setWithdrawingId(withdrawingId === t.id ? null : t.id);
                        }}
                      >
                        Withdraw
                      </button>
                    )}
                    {user && myStakes[t.id] && !isSimulationMode() && (
                      <button
                        className="btn-primary text-xs"
                        disabled={busy || !pendingByTag[t.id]}
                        onClick={() => claimTag(t.id, t.slug)}
                      >
                        {busy ? "…" : pendingByTag[t.id] ? `Claim ${formatToken(pendingByTag[t.id], "")}` : "Claim"}
                      </button>
                    )}
                    {user && (
                      <button
                        className="btn-secondary text-xs"
                        onClick={() => {
                          setWithdrawingId(null);
                          setStakingId(stakingId === t.id ? null : t.id);
                        }}
                      >
                        {stakingId === t.id ? "Cancel" : "Stake"}
                      </button>
                    )}
                  </div>
                );
              })()}
              {user && myStakes[t.id] && stakedAtByTag[t.id] !== undefined && (
                <div className="mt-1">
                  <UnstakeFeeNotice
                    feeBps={estimateUnstakeFee(myStakes[t.id]!.amount, stakedAtByTag[t.id]!).feeBps}
                  />
                </div>
              )}
              {revealRendered === t.id && (
                <div
                  className={cn(
                    "mt-3 flex flex-wrap items-center gap-2 transition-opacity duration-200 motion-safe:transition-[opacity,transform]",
                    revealVisible
                      ? "opacity-100 motion-safe:translate-y-0"
                      : "opacity-0 motion-safe:-translate-y-1",
                  )}
                >
                  <input
                    type="number"
                    min={1}
                    className="input min-w-0 flex-1"
                    value={stakeAmount}
                    onChange={(e) =>
                      setStakeAmount(Math.max(0, Number(e.target.value)))
                    }
                    aria-label="Stake amount"
                  />
                  <button
                    className="btn-primary shrink-0 text-sm"
                    disabled={busy}
                    onClick={() => stake(t.id, t.slug)}
                  >
                    {busy ? "…" : "Confirm"}
                  </button>
                </div>
              )}
              {withdrawRevealRendered === t.id && myStakes[t.id] && (
                <div
                  className={cn(
                    "mt-3 space-y-2 transition-opacity duration-200 motion-safe:transition-[opacity,transform]",
                    withdrawRevealVisible
                      ? "opacity-100 motion-safe:translate-y-0"
                      : "opacity-0 motion-safe:-translate-y-1",
                  )}
                >
                  {/* A disabled number input here (the amount isn't
                      editable — withdrawal is always the full stake)
                      used to sit in the same row as Confirm/Cancel, but
                      this sidebar is too narrow for all three to share a
                      row: the input's flex-basis got squeezed down to a
                      couple of characters, showing a truncated "3"
                      instead of "365.86". Plain text on its own line has
                      no width to clip against. */}
                  <p className="text-sm text-slate">
                    Withdraw all {formatToken(myStakes[t.id]!.amount, TOKEN_SYMBOL)}?
                  </p>
                  <div className="grid grid-cols-2 gap-2">
                    <button
                      className="btn-primary text-sm"
                      disabled={busy}
                      onClick={() => withdraw(t.id, t.slug)}
                    >
                      {busy ? "…" : "Confirm"}
                    </button>
                    <button
                      className="btn-secondary text-sm"
                      disabled={busy}
                      onClick={() => setWithdrawingId(null)}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}

      {user ? (
        <div className="flex items-center gap-2 border-t border-hairline pt-3">
          <input
            className="input"
            placeholder="Suggest a tag (e.g. lending)"
            value={newTag}
            maxLength={40}
            onChange={(e) => setNewTag(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && suggest()}
          />
          <button
            className="btn-secondary shrink-0"
            disabled={busy || !newTag.trim()}
            onClick={suggest}
          >
            Add
          </button>
        </div>
      ) : (
        <p className="border-t border-hairline pt-3 text-xs text-slate-steel">
          Sign in to stake or suggest tags.
        </p>
      )}
    </section>
  );
}
