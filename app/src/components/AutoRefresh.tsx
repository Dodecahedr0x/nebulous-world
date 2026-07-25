"use client";

import { useAutoRefresh } from "@/hooks/useAutoRefresh";

/** Renders nothing — mount once on a `force-dynamic` page to keep its
    server-fed amounts (stats, leaderboards, metrics) live. See useAutoRefresh. */
export function AutoRefresh({ intervalMs }: { intervalMs?: number }) {
  useAutoRefresh(intervalMs);
  return null;
}
