"use client";

import { shortAddress } from "@/lib/utils";
import { useLiveQuery } from "@/hooks/useLiveQuery";
import type { XpLeaderboardEntry } from "@/lib/indexerClient";

const LEADERBOARD_POLL_MS = 20000;

async function fetchLeaderboard(): Promise<XpLeaderboardEntry[]> {
  try {
    const res = await fetch("/api/xp/leaderboard");
    const json = await res.json();
    return json.ok ? json.data : [];
  } catch {
    return [];
  }
}

/**
 * Top 10 wallets by lifetime XP — a public, cosmetic ranking (never derived
 * from vote weight or stake). Shown on the profile page right below the
 * signed-in user's own level card, so a visitor sees both where they stand
 * personally and how that compares platform-wide.
 */
export function XpLeaderboard({ currentUserId }: { currentUserId?: string }) {
  const { data: entries } = useLiveQuery(fetchLeaderboard, [], { intervalMs: LEADERBOARD_POLL_MS });

  return (
    <section className="card space-y-3 p-6">
      <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">Leaderboard</h2>
      {entries === null ? (
        <p className="text-sm text-slate">Loading…</p>
      ) : entries.length === 0 ? (
        <p className="text-sm text-slate">No one has earned XP yet — be the first.</p>
      ) : (
        <ol className="space-y-2">
          {entries.map((entry, i) => (
            <li
              key={entry.userId}
              className={`flex items-center justify-between gap-3 rounded-lg border p-3 ${
                entry.userId === currentUserId
                  ? "border-cobalt/60 bg-indigo-soft"
                  : "border-hairline"
              }`}
            >
              <div className="flex min-w-0 items-center gap-3">
                <span className="w-5 shrink-0 tabular-nums text-slate-steel">{i + 1}</span>
                <span className="truncate text-sm font-medium text-ink">
                  {entry.handle ?? shortAddress(entry.wallet)}
                </span>
                <span className="chip chip-active font-mono text-[11px]">
                  Lv {entry.level} · {entry.title}
                </span>
              </div>
              <span className="font-mono text-xs tabular-nums text-slate-steel">{entry.xp} XP</span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
