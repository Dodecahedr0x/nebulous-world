"use client";

import { useCallback, useEffect, useRef, useState } from "react";

// This app has no push-based subscription to on-chain/indexer state (see
// AGENTS.md — the browser never opens its own RPC/websocket connection,
// everything is proxied through the indexer's REST API). Short-interval
// polling that pauses in background tabs and refires immediately on
// refocus is the closest practical substitute: every "amount" on screen
// should feel current within a few seconds, not just on page load.
const DEFAULT_INTERVAL_MS = 8000;

export interface UseLiveQueryOptions {
  /** How often to re-run `fetcher` while the tab is visible. */
  intervalMs?: number;
  /** Skip fetching entirely (e.g. no wallet connected yet). */
  enabled?: boolean;
}

export interface LiveQueryResult<T> {
  data: T | null;
  loading: boolean;
  error: Error | null;
  /** Re-runs `fetcher` immediately, outside the regular interval. */
  refresh: () => void;
}

/**
 * Runs `fetcher()` on mount, on every `deps` change, on a fixed interval,
 * and immediately whenever the tab regains focus/visibility — a live-ish
 * query for data this app can only pull, not have pushed to it. `deps`
 * works exactly like `useEffect`'s dependency array (same call site must
 * always pass the same length/order).
 */
export function useLiveQuery<T>(
  fetcher: () => Promise<T>,
  deps: unknown[],
  { intervalMs = DEFAULT_INTERVAL_MS, enabled = true }: UseLiveQueryOptions = {},
): LiveQueryResult<T> {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [nonce, setNonce] = useState(0);
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;

  const refresh = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    if (!enabled) {
      setData(null);
      setLoading(false);
      setError(null);
      return;
    }

    let cancelled = false;

    async function run() {
      setLoading(true);
      try {
        const result = await fetcherRef.current();
        if (!cancelled) {
          setData(result);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err : new Error(String(err)));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    run();
    const interval = setInterval(() => {
      if (document.visibilityState === "visible") run();
    }, intervalMs);

    function onWake() {
      if (document.visibilityState === "visible") run();
    }
    document.addEventListener("visibilitychange", onWake);
    window.addEventListener("focus", onWake);

    return () => {
      cancelled = true;
      clearInterval(interval);
      document.removeEventListener("visibilitychange", onWake);
      window.removeEventListener("focus", onWake);
    };
    // `deps` (caller-controlled, fixed shape per call site) drives refetch
    // the same way it would for a plain useEffect — see this hook's doc
    // comment. `nonce` powers manual `refresh()` calls.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, intervalMs, nonce, ...deps]);

  return { data, loading, error, refresh };
}
