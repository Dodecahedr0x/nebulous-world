"use client";

import { useEffect, useMemo, useState } from "react";
import { useAuth } from "@/components/providers/AuthProvider";
import { useToast } from "@/components/ui/Toaster";
import { useNebDlmmSwap } from "@/hooks/useNebDlmmSwap";
import { useWalletBalances } from "@/hooks/useWalletBalances";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { formatToken, splitValueUnit } from "@/lib/utils";
import type { NebPoolStatus } from "@/lib/indexerClient";
import { ConnectButton } from "@/components/ConnectButton";

const PRESETS = [10, 50, 100, 500];

/** Same label/value typography as MetricTrendCard's tile header (see
    components/explore/MetricTrendCard.tsx) — "resembles platform activity
    stats" per design intent, just without that card's trend chart, since
    the DLMM pool this reads from doesn't have a purchase-history table to
    chart a series from (see the old PoolAnalytics component's doc comment,
    now folded into this panel). */
function PoolStatTile({ label, value }: { label: string; value: string }) {
  const [amount, unit] = splitValueUnit(value);
  return (
    <div className="rounded-lg border border-hairline p-4">
      <div className="text-caption font-semibold uppercase tracking-wide text-slate">{label}</div>
      <div className="mt-1 flex flex-wrap items-baseline gap-x-1.5">
        <span className="text-heading-sm font-semibold tabular-nums text-ink">{amount}</span>
        {unit && <span className="text-subheading font-medium text-slate">{unit}</span>}
      </div>
    </div>
  );
}

export function BuyPanel() {
  const { user } = useAuth();
  const toast = useToast();
  const { buy } = useNebDlmmSwap();
  const [pool, setPool] = useState<NebPoolStatus | null | undefined>(undefined);
  const [usdcAmount, setUsdcAmount] = useState(50);
  const [busy, setBusy] = useState(false);
  const balances = useWalletBalances(pool?.nebMint ?? null, pool?.usdcMint ?? null);

  async function refresh() {
    const res = await fetch("/api/pool");
    const json = await res.json();
    setPool(json.ok ? json.data.pool : null);
  }

  useEffect(() => {
    refresh().catch(() => setPool(null));
  }, []);

  const quote = useMemo(() => {
    if (!pool || usdcAmount <= 0) return null;
    return usdcAmount / pool.price;
  }, [pool, usdcAmount]);

  async function handleBuy() {
    if (usdcAmount <= 0) return;
    setBusy(true);
    try {
      const { txSig, nebOut } = await buy(usdcAmount);
      toast.success(
        `Bought ${formatToken(nebOut, TOKEN_SYMBOL)} — tx confirmed`,
        txSig ? { txSig } : undefined,
      );
      await refresh();
      balances.refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Purchase failed");
    } finally {
      setBusy(false);
    }
  }

  if (pool === undefined) {
    return <section className="card p-6 text-sm text-slate">Loading pool…</section>;
  }

  if (pool === null) {
    return (
      <section className="card p-6 text-sm text-slate">
        {TOKEN_SYMBOL} isn&apos;t tradable yet — the launch pool hasn&apos;t been created.
      </section>
    );
  }

  return (
    <section className="card space-y-4 p-6">
      <div>
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">
          Buy {TOKEN_SYMBOL}
        </h2>
        <p className="mt-1 text-xs text-slate-steel">
          Swaps USDC for {TOKEN_SYMBOL} directly against the public NEB/USDC Meteora DLMM pool.
        </p>
      </div>

      {/* Live NEB/USDC Meteora DLMM pool indicators — proxied from the
          indexer (see lib/indexerClient.ts's fetchPoolStatus via /api/pool
          above), not a DB cache. Formerly its own "Pool analytics" section;
          folded in here since it's the same pool this panel already swaps
          against, and there's nothing else on the page that needs it. */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <PoolStatTile label="Price" value={`${pool.price.toFixed(2)} USDC`} />
        <PoolStatTile label={`${TOKEN_SYMBOL} in pool`} value={formatToken(pool.nebReserve, TOKEN_SYMBOL)} />
        <PoolStatTile label="USDC in pool" value={pool.usdcReserve.toFixed(2)} />
      </div>

      {!user ? (
        <div className="space-y-2">
          <p className="text-sm text-slate">Sign in to buy.</p>
          <ConnectButton />
        </div>
      ) : (
        <>
          {balances.neb != null && balances.usdc != null && (
            <div className="flex justify-between text-xs text-slate-steel">
              <span className="tabular-nums">Your balance: {formatToken(balances.neb, TOKEN_SYMBOL)}</span>
              <span className="tabular-nums">{balances.usdc.toFixed(2)} USDC</span>
            </div>
          )}
          <div className="flex flex-wrap gap-2">
            {PRESETS.map((p) => (
              <button
                key={p}
                onClick={() => setUsdcAmount(p)}
                className={`chip ${usdcAmount === p ? "chip-active" : ""}`}
              >
                {p} USDC
              </button>
            ))}
          </div>
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              step={1}
              className="input"
              value={usdcAmount}
              onChange={(e) => setUsdcAmount(Math.max(0, Number(e.target.value)))}
              aria-label="USDC amount"
            />
            <span className="text-sm text-slate">USDC</span>
          </div>
          {quote != null && (
            <p className="text-xs tabular-nums text-slate-steel">
              ≈ {formatToken(quote, TOKEN_SYMBOL)} at the current price
            </p>
          )}
          <button
            className="btn-primary w-full"
            disabled={busy || usdcAmount <= 0}
            onClick={handleBuy}
          >
            {busy ? "Buying…" : `Buy for ${usdcAmount.toFixed(2)} USDC`}
          </button>
        </>
      )}
    </section>
  );
}
