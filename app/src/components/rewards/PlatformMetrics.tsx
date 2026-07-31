import { formatToken, formatNumber, cn } from "@/lib/utils";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { MetricTrendCard, type TrendPoint } from "@/components/explore/MetricTrendCard";
import type { PlatformStats } from "@/lib/indexerClient";

/**
 * Platform-wide activity — apps, tags, votes cast, and stake, alongside
 * page-view traffic — each with a trend sparkline. Used to live on the
 * Explore page, but it's a snapshot of the whole product, not something
 * that helps you find or compare a specific app/tag the way the maps do —
 * moved here to sit with the rest of the product's "read the numbers"
 * surfaces, leaving Explore to just the maps. Pool-specific numbers (price,
 * reserves) live in BuyPanel instead, next to the swap form they describe.
 *
 * Rendered in two different containers now: Rewards' and Rankings' both
 * full-width max-w-7xl pages (`wide=true`) — see rewards/page.tsx and
 * rankings/page.tsx. The `wide` prop switches the column count up on large
 * screens so 5-6 tiles don't look sparse/stretched in that width; the
 * `false` default stays a tighter 3-across for any future narrower caller.
 */
export function PlatformMetrics({
  stats,
  appsTrend,
  tagsTrend,
  votesTrend,
  stakeTrend,
  viewsTrend,
  revenueTrend,
  revenueTotal,
  wide = false,
}: {
  stats: PlatformStats;
  appsTrend: TrendPoint[];
  tagsTrend: TrendPoint[];
  votesTrend: TrendPoint[];
  stakeTrend: TrendPoint[];
  viewsTrend: TrendPoint[];
  /** Both omitted entirely (not just empty/0) on pages that don't fetch
      this data — the tile only renders when `revenueTrend` is passed, so
      it stays opt-in rather than a sixth tile appearing everywhere with a
      flat-zero placeholder. Pre-scaled by the caller (divided by the vote
      token's decimals), same as every trend point already passed in here —
      this component never does raw-u64 unit math itself. */
  revenueTrend?: TrendPoint[];
  revenueTotal?: number;
  wide?: boolean;
}) {
  return (
    <section className="card space-y-4 p-6">
      <div>
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate">
          Platform activity
        </h2>
        <p className="mt-1 text-xs text-slate-steel">
          Apps, tags, votes, and stake across nebulous.world, plus page-view traffic.
        </p>
      </div>

      {/* 3 columns by default; `wide` opts into a 6-across grid on large
          screens — a clean fit whether this renders 5 tiles (Rankings, no
          revenue tile: 5 of 6 columns used) or 6 (Rewards, with the revenue
          tile: exact fit, and also exactly 2 even rows of 3 at the sm
          breakpoint either way). */}
      <div className={cn("grid grid-cols-2 gap-4 sm:grid-cols-5", wide && "lg:grid-cols-6")}>
        <MetricTrendCard label="Apps" value={formatNumber(stats.totalApps)} data={appsTrend} />
        <MetricTrendCard label="Tags" value={formatNumber(stats.totalTags)} data={tagsTrend} />
        <MetricTrendCard
          label="Votes cast"
          value={formatToken(stats.totalVoteWeight, TOKEN_SYMBOL)}
          data={votesTrend}
          valueKind="token"
        />
        <MetricTrendCard
          label="Staked"
          value={formatToken(stats.totalStake, TOKEN_SYMBOL)}
          data={stakeTrend}
          valueKind="token"
        />
        <MetricTrendCard label="Page views" value={formatNumber(stats.totalViews)} data={viewsTrend} />
        {revenueTrend && (
          <MetricTrendCard
            label="Revenue distributed"
            value={formatToken(revenueTotal ?? 0, TOKEN_SYMBOL)}
            data={revenueTrend}
            valueKind="token"
          />
        )}
      </div>
    </section>
  );
}
