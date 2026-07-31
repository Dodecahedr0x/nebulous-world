"use client";

import { useEffect, useId, useRef, useState } from "react";
import Image from "next/image";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useDigest } from "@/hooks/useDigest";
import { TOKEN_SYMBOL } from "@/lib/constants";
import type {
  DigestDTO,
  DigestRankMoveItem,
  DigestRewardItem,
  DigestStreakItem,
} from "@/lib/types";
import { cn, formatToken } from "@/lib/utils";

/**
 * The navbar's "since you were last here" bell — claimable rewards, rank
 * moves on staked apps, and streak status, in that fixed order (the array
 * order the indexer returns IS the render order; this component never
 * re-sorts). See docs/plans/2026-08-01-staker-digest-design.md.
 *
 * Only meaningful when signed in — it renders nothing at all without a
 * digest, so `Navbar` can mount it unconditionally behind `connected`.
 */
export function DigestBell() {
  const { digest, markSeen } = useDigest();
  const pathname = usePathname();
  const [open, setOpen] = useState(false);
  // The panel renders a SNAPSHOT taken at open time, not the live query.
  // Opening advances the server watermark, and the background poll can fire
  // at any moment — without this freeze, rank moves would silently vanish
  // from under the reader's cursor mid-read. The next open picks up
  // whatever the watermark advance produced.
  const [frozen, setFrozen] = useState<DigestDTO | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const panelId = useId();
  const headingId = useId();

  const count = digest?.count ?? 0;

  // Escape dismisses and hands focus back to the bell that opened it —
  // otherwise a keyboard user is left with focus on a panel that no longer
  // exists, back at the top of the document.
  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      setOpen(false);
      buttonRef.current?.focus();
    }
    function onPointerDown(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("mousedown", onPointerDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("mousedown", onPointerDown);
    };
  }, [open]);

  // Moving focus into the panel on open is what makes the rows reachable by
  // keyboard in document order without tabbing back through the whole
  // header first.
  useEffect(() => {
    if (open) panelRef.current?.focus();
  }, [open]);

  // Any navigation (including clicking a row inside the panel) closes it,
  // same as the mobile nav dropdown.
  useEffect(() => {
    setOpen(false);
  }, [pathname]);

  function toggle() {
    if (open) {
      setOpen(false);
      return;
    }
    setFrozen(digest);
    setOpen(true);
    markSeen();
  }

  if (!digest) return null;

  const shown = frozen ?? digest;

  return (
    <div ref={containerRef} className="relative">
      <button
        ref={buttonRef}
        type="button"
        onClick={toggle}
        aria-expanded={open}
        aria-controls={panelId}
        aria-haspopup="true"
        aria-label={count > 0 ? `Updates, ${count} new` : "Updates"}
        className="relative rounded-navitem p-1.5 text-slate transition-colors duration-150 ease-out hover:text-ink"
      >
        <svg
          className="h-5 w-5"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M15 17h5l-1.4-1.4A2 2 0 0 1 18 14.2V11a6 6 0 1 0-12 0v3.2a2 2 0 0 1-.6 1.4L4 17h5m6 0v1a3 3 0 1 1-6 0v-1m6 0H9"
          />
        </svg>
        {count > 0 && (
          // aria-hidden: the count is already in the button's accessible
          // name above, so announcing it twice would just be noise.
          <span
            aria-hidden="true"
            className="absolute -right-0.5 -top-0.5 grid h-4 min-w-4 place-items-center rounded-full bg-cobalt px-1 text-[10px] font-semibold tabular-nums leading-none text-cream"
          >
            {count}
          </span>
        )}
      </button>

      {open && (
        <div
          ref={panelRef}
          id={panelId}
          role="region"
          aria-labelledby={headingId}
          tabIndex={-1}
          // Below sm the bell isn't the rightmost thing in the header
          // (Connect + the nav toggle sit to its right), so a bell-anchored
          // 20rem panel would hang off the left edge of a phone. Pin it to
          // the viewport gutters there instead, and only anchor it to the
          // bell once there's room.
          className="animate-fade-in fixed inset-x-4 top-[4.5rem] z-50 overflow-hidden rounded-card border border-hairline bg-cream shadow-hover focus:outline-none sm:absolute sm:inset-x-auto sm:right-0 sm:top-full sm:mt-2 sm:w-80"
        >
          <h2
            id={headingId}
            className="border-b border-hairline px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-slate-steel"
          >
            Since your last visit
          </h2>
          {shown.items.length === 0 ? (
            <p className="px-3 py-4 text-sm text-slate">
              You&rsquo;re all up to date — nothing new since your last visit.
            </p>
          ) : (
            <ul className="max-h-96 divide-y divide-hairline overflow-y-auto">
              {shown.items.map((item, i) => (
                <li key={item.kind === "streak" ? "streak" : `${item.kind}:${item.appId}:${i}`}>
                  {item.kind === "reward" ? (
                    <RewardRow item={item} />
                  ) : item.kind === "rank_move" ? (
                    <RankMoveRow item={item} />
                  ) : (
                    <StreakRow item={item} />
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

const ROW = "flex w-full items-center gap-2.5 px-3 py-2.5 text-left transition-colors duration-150 ease-out";
const ROW_LINK = `${ROW} hover:bg-mist`;

/** 24px app icon, or the app's initial on an Indigo-tinted well when the
    app has no image — same fallback treatment as AppCard's hero. */
function AppIcon({ src, name }: { src: string | null; name: string }) {
  if (!src) {
    return (
      <span className="grid h-6 w-6 shrink-0 place-items-center rounded-image bg-mist text-[11px] font-bold text-cobalt">
        {name.charAt(0).toUpperCase() || "?"}
      </span>
    );
  }
  return (
    <Image
      src={src}
      alt=""
      width={24}
      height={24}
      className="h-6 w-6 shrink-0 rounded-image object-cover"
    />
  );
}

/** Claimable revenue on one staked app, summed across `epochCount` settled
    epochs. `amount` arrives in whole-token units already (see lib/types.ts),
    so it goes straight to `formatToken` with no scaling — same as MyStakes
    renders its `stakedAmount`. */
function RewardRow({ item }: { item: DigestRewardItem }) {
  const amount = item.amount;
  return (
    <Link href="/rewards" className={ROW_LINK}>
      <AppIcon src={item.appIconUrl} name={item.appName} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-ink">{item.appName}</span>
        <span className="block text-xs tabular-nums text-slate">
          {formatToken(amount, TOKEN_SYMBOL)} claimable
          <span className="text-slate-steel">
            {" · "}
            {item.epochCount} {item.epochCount === 1 ? "epoch" : "epochs"}
          </span>
        </span>
      </span>
    </Link>
  );
}

/**
 * A leaderboard-position move on a staked app. Direction is carried THREE
 * ways — an arrow glyph, a signed number, and colour — because colour alone
 * would fail WCAG 2.1 AA (1.4.1 Use of Color). The visual composition is
 * hidden from assistive tech in favour of one plain-language sentence,
 * since "↑ +3 #7 → #4" read aloud verbatim is gibberish.
 */
function RankMoveRow({ item }: { item: DigestRankMoveItem }) {
  const up = item.delta > 0;
  const magnitude = Math.abs(item.delta);
  return (
    <Link href={`/app/${item.appSlug}`} className={ROW_LINK}>
      <AppIcon src={item.appIconUrl} name={item.appName} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-ink">{item.appName}</span>
        <span aria-hidden="true" className="block text-xs tabular-nums text-slate">
          <span className={cn("font-medium", up ? "text-forest" : "text-negative")}>
            {up ? "↑" : "↓"} {up ? "+" : "−"}
            {magnitude}
          </span>
          <span className="text-slate-steel">
            {" · #"}
            {item.from} → #{item.to}
          </span>
        </span>
        <span className="sr-only">
          {`Moved ${up ? "up" : "down"} ${magnitude} ${magnitude === 1 ? "position" : "positions"}, from rank ${item.from} to rank ${item.to}`}
        </span>
      </span>
    </Link>
  );
}

/**
 * Streak status. The daily bonus is auto-awarded by any qualifying action —
 * there is no button, and nothing to "claim" by showing up — so the copy
 * names the actions that actually earn it instead of implying an
 * affordance that doesn't exist.
 */
function StreakRow({ item }: { item: DigestStreakItem }) {
  return (
    <div className={ROW}>
      <span
        aria-hidden="true"
        className="grid h-6 w-6 shrink-0 place-items-center rounded-image bg-indigo-soft text-[11px] font-bold tabular-nums text-cobalt"
      >
        {item.streakDays}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-ink">
          {item.streakDays}-day streak
          <span className="ml-1.5 text-xs font-normal tabular-nums text-slate-steel">
            best {item.bestDays}
          </span>
        </span>
        <span className="block text-xs text-slate">
          {item.bonusClaimedToday
            ? "Today's bonus is already in — nice."
            : "Vote, stake, or suggest a tag to keep your streak."}
        </span>
      </span>
    </div>
  );
}
