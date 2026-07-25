"use client";

import { useEffect, useRef } from "react";

const DEFAULT_INTERVAL_MS = 8000;

/**
 * Like `useEffect`, but re-runs `effect` on an interval (paused while the
 * tab is hidden, fired immediately on refocus) in addition to on mount and
 * dependency changes — for effect bodies that set several pieces of local
 * state from one async load (see VotePanel/TagStakePanel/ClaimRewards/
 * MyStakes) and so don't fit `useLiveQuery`'s single-`data` shape. `effect`
 * gets an `isCancelled()` check in place of the usual cleanup-flag closure,
 * since one run can still be in flight when the next poll fires.
 */
export function usePollingEffect(
  effect: (isCancelled: () => boolean) => void | Promise<void>,
  deps: unknown[],
  intervalMs = DEFAULT_INTERVAL_MS,
) {
  const effectRef = useRef(effect);
  effectRef.current = effect;

  useEffect(() => {
    let cancelled = false;
    const isCancelled = () => cancelled;

    effectRef.current(isCancelled);
    const interval = setInterval(() => {
      if (document.visibilityState === "visible") effectRef.current(isCancelled);
    }, intervalMs);

    function onWake() {
      if (document.visibilityState === "visible") effectRef.current(isCancelled);
    }
    document.addEventListener("visibilitychange", onWake);
    window.addEventListener("focus", onWake);

    return () => {
      cancelled = true;
      clearInterval(interval);
      document.removeEventListener("visibilitychange", onWake);
      window.removeEventListener("focus", onWake);
    };
    // `deps` is caller-controlled with a fixed shape per call site, same
    // convention as useLiveQuery — see its doc comment.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [intervalMs, ...deps]);
}
