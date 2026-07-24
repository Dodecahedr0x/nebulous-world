"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useAuth } from "@/components/providers/AuthProvider";
import { ConnectButton } from "@/components/ConnectButton";
import { XpLeaderboard } from "@/components/profile/XpLeaderboard";
import { timeAgo } from "@/lib/utils";
import type { UserXp, XpActivityEntry, XpTaskKind } from "@/lib/indexerClient";

// Kept in sync by hand with the XP_* consts in indexer/src/handlers/xp.rs —
// this is presentation only (label/href), the point values here are for
// display, the actual award still happens server-side.
const XP_TASKS: { kind: XpTaskKind; xp: number; label: string; href: string }[] = [
  { kind: "submit_app", xp: 100, label: "List an app", href: "/?create=1" },
  { kind: "suggest_tag", xp: 40, label: "Suggest a tag on an app", href: "/" },
  { kind: "vote", xp: 20, label: "Vote for an app", href: "/" },
  { kind: "stake", xp: 30, label: "Stake on a tag", href: "/" },
];

function describeEvent(event: XpActivityEntry): string {
  switch (event.kind) {
    case "submit_app":
      return `Submitted ${event.appName ?? "an app"}`;
    case "suggest_tag":
      return `Suggested #${event.tagName ?? "a tag"} on ${event.appName ?? "an app"}`;
    case "vote":
      return `Voted on ${event.appName ?? "an app"}`;
    case "stake":
      return `Staked on #${event.tagName ?? "a tag"} (${event.appName ?? "an app"})`;
    case "daily_bonus":
      return "Daily bonus";
    default:
      return event.kind;
  }
}

export function XpProgress() {
  const { user, loading: authLoading } = useAuth();
  const [xp, setXp] = useState<UserXp | null | undefined>(undefined);
  const [activity, setActivity] = useState<XpActivityEntry[] | null>(null);

  useEffect(() => {
    if (!user) {
      setXp(null);
      setActivity(null);
      return;
    }

    let cancelled = false;

    async function load() {
      const [xpRes, activityRes] = await Promise.all([
        fetch("/api/xp/me").then((r) => r.json()),
        fetch("/api/xp/me/activity").then((r) => r.json()),
      ]);
      if (cancelled) return;
      if (xpRes.ok) setXp(xpRes.data);
      if (activityRes.ok) setActivity(activityRes.data);
    }

    load().catch(() => {
      if (!cancelled) {
        setXp(null);
        setActivity([]);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [user]);

  if (authLoading) {
    return (
      <section className="card p-6">
        <p className="text-sm text-slate">Loading your profile…</p>
      </section>
    );
  }

  if (!user) {
    return (
      <section className="card space-y-3 p-6">
        <p className="text-sm text-slate">Sign in to see your level and activity.</p>
        <ConnectButton />
      </section>
    );
  }

  if (xp === undefined) {
    return (
      <section className="card p-6">
        <p className="text-sm text-slate">Loading your profile…</p>
      </section>
    );
  }

  if (xp === null) {
    return (
      <section className="card p-6">
        <p className="text-sm text-slate">Couldn&apos;t load your profile. Try refreshing.</p>
      </section>
    );
  }

  const pct = Math.round(xp.progress * 100);
  const remainingTasks = XP_TASKS.filter((t) => !xp.xpEarnedToday.includes(t.kind));

  return (
    <div className="space-y-6">
      <section className="card space-y-2 p-4">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-slate">
          Earn more XP today
        </h2>
        {remainingTasks.length === 0 ? (
          <p className="text-sm text-slate">
            You&apos;ve earned XP for everything today — nice work. Come back tomorrow for more.
          </p>
        ) : (
          <ul className="divide-y divide-hairline">
            {remainingTasks.map((t) => (
              <li key={t.kind} className="flex items-center justify-between gap-3 py-2 first:pt-0 last:pb-0">
                <span className="text-sm text-ink">{t.label}</span>
                <Link
                  href={t.href}
                  className="flex items-center gap-1 text-sm font-medium text-cobalt hover:underline"
                >
                  +{t.xp} XP <span aria-hidden="true">→</span>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="card space-y-4 p-6">
        <div className="flex items-center justify-between">
          <span className="chip chip-active font-mono tabular-nums">
            Lv {xp.level} · {xp.title}
          </span>
          <span className="font-mono text-xs tabular-nums text-slate-steel">
            {xp.xpIntoLevel} / {xp.xpForNextLevel} XP
          </span>
        </div>
        <div className="h-2 w-full overflow-hidden rounded-pill bg-mist">
          <div
            className="h-full rounded-pill bg-cobalt transition-[width] duration-[250ms] ease-out"
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Stat label="Apps submitted" value={xp.appsSubmitted} />
          <Stat label="Tags suggested" value={xp.tagsSuggested} />
          <Stat label="Votes cast" value={xp.votesCast} />
          <Stat label="Tags staked" value={xp.stakesMade} />
        </div>
      </section>

      <XpLeaderboard currentUserId={xp.userId} />

      <section className="card space-y-3 p-6">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">Activity</h2>
        {activity === null ? (
          <p className="text-sm text-slate">Loading…</p>
        ) : activity.length === 0 ? (
          <p className="text-sm text-slate">
            No activity yet.{" "}
            <Link href="/" className="font-medium text-cobalt hover:underline">
              Discover an app
            </Link>{" "}
            to start earning XP.
          </p>
        ) : (
          // Capped height + scroll — the API returns up to 50 events (see
          // indexer/src/handlers/xp.rs's get_activity), which would
          // otherwise stretch this section far past the rest of the page.
          <ul className="max-h-96 space-y-2 overflow-y-auto pr-1">
            {activity.map((event) => (
              <li
                key={event.id}
                className="flex items-center justify-between gap-3 rounded-lg border border-hairline p-3"
              >
                <div>
                  <div className="text-sm text-ink">{describeEvent(event)}</div>
                  <div className="text-xs text-slate-steel">{timeAgo(event.createdAt)}</div>
                </div>
                <span className="font-mono text-xs tabular-nums text-forest">+{event.amount} XP</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div>
      <div className="text-xs text-slate-steel">{label}</div>
      <div className="font-mono text-lg font-semibold tabular-nums text-ink">{value}</div>
    </div>
  );
}
