"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

const DEFAULT_INTERVAL_MS = 15000;

/**
 * Calls `router.refresh()` on an interval (paused in background tabs, and
 * fired immediately on refocus) — the equivalent of `useLiveQuery` for a
 * `force-dynamic` Server Component page, whose data can only be refreshed by
 * re-running the whole route rather than a single client fetch. Cheap tiles
 * fed straight from server props (platform stats, leaderboards, app metrics)
 * use this instead of duplicating a client-side fetch of their own.
 */
export function useAutoRefresh(intervalMs = DEFAULT_INTERVAL_MS) {
  const router = useRouter();

  useEffect(() => {
    const interval = setInterval(() => {
      if (document.visibilityState === "visible") router.refresh();
    }, intervalMs);

    function onWake() {
      if (document.visibilityState === "visible") router.refresh();
    }
    document.addEventListener("visibilitychange", onWake);
    window.addEventListener("focus", onWake);

    return () => {
      clearInterval(interval);
      document.removeEventListener("visibilitychange", onWake);
      window.removeEventListener("focus", onWake);
    };
  }, [router, intervalMs]);
}
