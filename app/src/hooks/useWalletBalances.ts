"use client";

import { useCallback } from "react";
import { useWallet } from "@solana/wallet-adapter-react";
import { apiGet } from "@/lib/txClient";
import { useLiveQuery } from "@/hooks/useLiveQuery";

export interface WalletBalances {
  neb: number | null;
  usdc: number | null;
  refresh: () => void;
}

async function fetchBalance(owner: string, mint: string): Promise<number> {
  try {
    const { uiAmountString } = await apiGet<{ uiAmountString: string }>(
      `/api/balances/${owner}/${mint}`,
    );
    return Number(uiAmountString);
  } catch {
    // No ATA yet (never held the token) — treat as a zero balance.
    return 0;
  }
}

// Polled rather than fetched once — a balance can move from an external
// transfer or swap the app itself didn't initiate, so it can't rely solely
// on callers remembering to call `refresh()`.
const BALANCE_INTERVAL_MS = 8000;

/** The connected wallet's NEB and/or USDC balance. Either mint can be
    omitted (`null`) independently — e.g. the navbar only wants NEB — in
    which case that side just stays `null` rather than blocking the other. */
export function useWalletBalances(nebMint: string | null, usdcMint: string | null): WalletBalances {
  const { publicKey } = useWallet();
  const owner = publicKey?.toBase58() ?? null;

  const neb = useLiveQuery(() => fetchBalance(owner!, nebMint!), [owner, nebMint], {
    enabled: !!owner && !!nebMint,
    intervalMs: BALANCE_INTERVAL_MS,
  });
  const usdc = useLiveQuery(() => fetchBalance(owner!, usdcMint!), [owner, usdcMint], {
    enabled: !!owner && !!usdcMint,
    intervalMs: BALANCE_INTERVAL_MS,
  });

  const refreshNeb = neb.refresh;
  const refreshUsdc = usdc.refresh;
  const refresh = useCallback(() => {
    refreshNeb();
    refreshUsdc();
  }, [refreshNeb, refreshUsdc]);

  return { neb: neb.data, usdc: usdc.data, refresh };
}
