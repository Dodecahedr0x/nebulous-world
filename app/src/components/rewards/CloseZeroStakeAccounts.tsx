"use client";

import { useState } from "react";
import { useWallet } from "@solana/wallet-adapter-react";
import { useToast } from "@/components/ui/Toaster";
import { useClosePositions } from "@/hooks/useClosePositions";
import { useLiveQuery } from "@/hooks/useLiveQuery";
import { isSimulationMode } from "@/lib/config";
import { formatToken } from "@/lib/utils";
import { apiGet } from "@/lib/txClient";
import type { CloseablePosition } from "@/lib/indexerClient";

const LAMPORTS_PER_SOL = 1_000_000_000;
const CLOSEABLE_POLL_MS = 15000;

/**
 * A fully-withdrawn `VotePosition`/`StakePosition` still sits on-chain
 * (nothing closes it automatically) until its owner reclaims the rent SOL
 * — this is that reclaim action. Only renders once there's actually
 * something to close, so it stays out of the way for everyone else on the
 * rewards page (unlike `ClaimRewards`, which is always relevant once
 * signed in).
 */
export function CloseZeroStakeAccounts() {
  const wallet = useWallet();
  const toast = useToast();
  const { closeVotePosition, closeTagStakePosition } = useClosePositions();

  const [closing, setClosing] = useState(false);
  const owner = wallet.publicKey?.toBase58() ?? null;

  const { data: positions, refresh: refreshPositions } = useLiveQuery(
    async () => {
      try {
        const { positions } = await apiGet<{ positions: CloseablePosition[] }>(
          `/api/wallet/${owner}/closeable-positions`,
        );
        return positions;
      } catch {
        return [];
      }
    },
    [owner],
    { enabled: !isSimulationMode() && !!owner, intervalMs: CLOSEABLE_POLL_MS },
  );

  if (!positions || positions.length === 0) return null;

  const totalLamports = positions.reduce((sum, p) => sum + p.lamports, 0);

  async function closeAll() {
    if (!positions) return;
    setClosing(true);
    let closed = 0;
    let failed = 0;
    let reclaimedLamports = 0;
    for (const p of positions) {
      try {
        await (p.kind === "vote" ? closeVotePosition(p.position) : closeTagStakePosition(p.position));
        closed++;
        reclaimedLamports += p.lamports;
      } catch {
        failed++;
      }
    }
    setClosing(false);

    if (closed > 0) {
      toast.success(
        `Closed ${closed} account${closed === 1 ? "" : "s"}, reclaiming ${formatToken(
          reclaimedLamports / LAMPORTS_PER_SOL,
          "SOL",
        )}${failed > 0 ? ` (${failed} failed — try again)` : ""}`,
      );
    } else {
      toast.error("Couldn't close any accounts — try again");
    }

    // Re-fetch rather than assume every close succeeded (a wallet rejection
    // mid-batch leaves some positions still open) — the component just
    // disappears on its own once the list comes back empty.
    refreshPositions();
  }

  return (
    <section className="card space-y-3 p-6">
      <div>
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">
          Reclaim unused rent
        </h2>
        <p className="mt-1 text-xs text-slate-steel">
          You have {positions.length} vote/stake account{positions.length === 1 ? "" : "s"} at zero
          balance — fully withdrawn, but still holding a small SOL rent deposit on-chain. Closing
          them refunds that deposit to whichever wallet originally paid it.
        </p>
      </div>
      <button className="btn-primary text-xs" disabled={closing} onClick={closeAll}>
        {closing
          ? "Closing…"
          : `Close ${positions.length} account${positions.length === 1 ? "" : "s"} (~${formatToken(
              totalLamports / LAMPORTS_PER_SOL,
              "SOL",
            )})`}
      </button>
    </section>
  );
}
