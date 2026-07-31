"use client";

import { useCallback } from "react";
import { usePathname } from "next/navigation";
import { useAuth } from "@/components/providers/AuthProvider";
import { useLiveQuery } from "@/hooks/useLiveQuery";
import type { DigestDTO } from "@/lib/types";

// The digest is a "what changed since your last visit" payload, not a live
// ticker: the trigger is a returning visit, not elapsed seconds. `pathname`
// in the dep array is what actually drives refetching — mount and every
// route change — and useLiveQuery's own focus/visibilitychange refire
// covers "came back to the tab". The interval is deliberately long (rather
// than absent, since useLiveQuery always sets one) so a tab parked open for
// an hour isn't stuck showing a stale badge, without polling like a chat
// app would.
const DIGEST_INTERVAL_MS = 300_000;

async function fetchDigest(): Promise<DigestDTO | null> {
  const res = await fetch("/api/digest");
  const json = await res.json();
  return json.ok ? (json.data as DigestDTO | null) : null;
}

export interface UseDigestResult {
  /** `null` while signed out, loading, or if the user has no digest yet. */
  digest: DigestDTO | null;
  /**
   * Advances the server-side watermark. Deliberately fire-and-forget and
   * deliberately does NOT refresh `digest` — the panel's contents must stay
   * put for the view the user is currently looking at; the advance shows up
   * on the next load. Errors are swallowed: a missed watermark write just
   * means the same items show up again, which is a far better failure than
   * an error toast over the navbar.
   */
  markSeen: () => void;
}

/** The signed-in user's "since you were last here" digest, for the navbar
    bell. Follows the `useUserLevel` shape — `useLiveQuery`, keyed on `user`,
    `enabled: !!user` — so signed-out callers get `null` and hide the bell. */
export function useDigest(): UseDigestResult {
  const { user } = useAuth();
  const pathname = usePathname();
  const { data } = useLiveQuery(fetchDigest, [user, pathname], {
    enabled: !!user,
    intervalMs: DIGEST_INTERVAL_MS,
  });

  const markSeen = useCallback(() => {
    void fetch("/api/digest/seen", { method: "POST" }).catch(() => {});
  }, []);

  return { digest: data, markSeen };
}
