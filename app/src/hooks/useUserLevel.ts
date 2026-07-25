"use client";

import { useAuth } from "@/components/providers/AuthProvider";
import { useLiveQuery } from "@/hooks/useLiveQuery";

export interface UserLevel {
  level: number;
  title: string;
}

const LEVEL_INTERVAL_MS = 15000;

async function fetchLevel(): Promise<UserLevel | null> {
  const res = await fetch("/api/xp/me");
  const json = await res.json();
  return json.ok && json.data ? { level: json.data.level, title: json.data.title } : null;
}

/** The signed-in user's XP level, for the navbar badge. `null` while signed
    out or loading — callers should just hide the badge in that case, same
    convention as `useWalletBalances`'s `neb: null`. */
export function useUserLevel(): UserLevel | null {
  const { user } = useAuth();
  const { data } = useLiveQuery(fetchLevel, [user], {
    enabled: !!user,
    intervalMs: LEVEL_INTERVAL_MS,
  });
  return data;
}
