import Image from "next/image";
import Link from "next/link";
import type { AppDTO } from "@/lib/types";
import { formatToken, formatNumber, hostname, cn, topStakedTag, formatDelta } from "@/lib/utils";
import { TagChip } from "@/components/app/TagChip";

// Caps the chips shown on a card — an app can carry arbitrarily many tags
// (see scripts/createAppsOnchain.ts), but the card itself has room for a
// handful. Highest-stake tags win the cut, sorted below rather than trusting
// callers to have already ordered `app.tags` that way.
const MAX_VISIBLE_TAGS = 5;

/** Compact metric with a label, plus an optional recent-change subtext
    ("+12%/7d") — green for a gain, red for a decline, matching the
    achromatic-except-accents convention the rest of the app's chip/error
    states already use (text-forest / text-negative). Omitted entirely (not
    "0%") when there's no baseline snapshot to compare against — see
    formatDelta's doc comment for why a genuine 0% still renders. */
function Stat({ label, value, deltaPct, intervalDays }: { label: string; value: string; deltaPct?: number | null; intervalDays?: number }) {
  const delta = intervalDays != null ? formatDelta(deltaPct ?? null, intervalDays) : null;
  return (
    <div className="flex flex-col">
      <span className="text-sm font-semibold tabular-nums text-ink">
        {value}
      </span>
      <span className="text-[10px] uppercase tracking-wide text-slate-steel">
        {label}
      </span>
      {delta && (
        <span
          className={cn(
            "text-[10px] tabular-nums",
            (deltaPct ?? 0) >= 0 ? "text-forest" : "text-negative",
          )}
        >
          {delta}
        </span>
      )}
    </div>
  );
}

export function AppCard({
  app,
  rank,
  preview = false,
}: {
  app: AppDTO;
  rank?: number;
  /** Renders as an inert div instead of a Link — used for the live preview
      in CreateAppForm, where the card isn't a real, navigable app yet. */
  preview?: boolean;
}) {
  const className = cn(
    "group flex flex-col overflow-hidden",
    preview ? "card" : "card-interactive",
  );
  // Apps have no onchain "category" — the corner badge shows the tag with
  // the most stake behind it instead, or nothing if the app has no tags.
  const topTag = topStakedTag(app.tags);

  const content = (
    <>
      {/* Hero image — the app's own OpenGraph image when available, so the
          card reads like a link preview rather than a bare list row. */}
      <div className="relative aspect-[1200/630] w-full shrink-0 overflow-hidden bg-mist">
        {app.iconUrl ? (
          <Image
            src={app.iconUrl}
            alt=""
            fill
            sizes="(min-width: 1024px) 33vw, (min-width: 640px) 50vw, 100vw"
            // Above-the-fold apps (top-ranked on the first screenful) load
            // eagerly so they're not the page's LCP bottleneck; the rest
            // lazy-load like any other next/image.
            priority={typeof rank === "number" && rank <= 3}
            className="object-cover ring-1 ring-inset ring-white/10 transition-transform duration-300 group-hover:scale-[1.03]"
          />
        ) : (
          <div className="grid h-full w-full place-items-center text-4xl font-bold text-violet">
            {app.name.charAt(0).toUpperCase() || "?"}
          </div>
        )}
        {typeof rank === "number" && (
          <span className="absolute left-2.5 top-2.5 grid h-6 w-6 place-items-center rounded-full bg-cream/80 text-[11px] font-bold text-ink">
            {rank}
          </span>
        )}
        {topTag && (
          <TagChip
            tag={topTag}
            className="absolute right-2.5 top-2.5 border-none bg-ivory/90"
          />
        )}
      </div>

      {/* Domain + title strip, mirroring how a shared link preview unfurls. */}
      <div className="border-t border-hairline bg-ivory/60 px-4 py-3">
        <span className="text-[11px] font-medium uppercase tracking-wide text-slate-steel">
          {hostname(app.url)}
        </span>
        <h3 className="mt-0.5 truncate text-base font-bold text-ink group-hover:text-cobalt">
          {app.name}
        </h3>
        <p className="mt-0.5 line-clamp-2 text-sm text-slate">
          {app.tagline || app.description}
        </p>
      </div>

      {app.tags.length > 0 && (
        <div className="flex flex-wrap gap-1.5 px-4 py-2">
          {[...app.tags]
            .sort((a, b) => b.stakeTotal - a.stakeTotal)
            .slice(0, MAX_VISIBLE_TAGS)
            .map((t) => (
              <TagChip key={t.id} tag={t} />
            ))}
        </div>
      )}

      <div className="mt-auto grid grid-cols-3 gap-2 border-t border-hairline p-4">
        <Stat
          label="Rank"
          value={app.rankScore.toFixed(2)}
          deltaPct={app.trend?.rankScorePct}
          intervalDays={app.trend?.intervalDays}
        />
        <Stat
          label="Staked"
          value={formatToken(app.stakeTotal, "")}
          deltaPct={app.trend?.stakeTotalPct}
          intervalDays={app.trend?.intervalDays}
        />
        <Stat
          label="Views"
          value={formatNumber(app.viewCount)}
          deltaPct={app.trend?.viewCountPct}
          intervalDays={app.trend?.intervalDays}
        />
      </div>
    </>
  );

  if (preview) {
    return <div className={className}>{content}</div>;
  }

  return (
    <Link href={`/app/${app.slug}`} className={className}>
      {content}
    </Link>
  );
}
