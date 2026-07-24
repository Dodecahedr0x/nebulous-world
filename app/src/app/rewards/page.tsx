import type { Metadata } from "next";
import {
  fetchPlatformStats,
  fetchPlatformMetricsHistory,
  fetchPlatformViewsTrend,
  fetchRevenueDistributedTrend,
} from "@/lib/indexerClient";
import { TOKEN_NAME, TOKEN_SYMBOL, SITE_URL } from "@/lib/constants";
import { config } from "@/lib/config";
import { BuyPanel } from "@/components/token/BuyPanel";
import { PlatformMetrics } from "@/components/rewards/PlatformMetrics";
import { ClaimRewards } from "@/components/rewards/ClaimRewards";
import { CloseZeroStakeAccounts } from "@/components/rewards/CloseZeroStakeAccounts";
import type { TrendPoint } from "@/components/explore/MetricTrendCard";
import { PageHeader } from "@/components/PageHeader";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Rewards",
  description: `Buy ${TOKEN_SYMBOL}, track the pool, and claim your vote/stake rewards — everything ${TOKEN_SYMBOL}-related on nebulous.world, in one place.`,
  alternates: { canonical: `${SITE_URL}/rewards` },
};

export default async function RewardsPage() {
  // The on-chain-derived series (apps/tags/votes/stake/revenue) comes from
  // the indexer, which is a separate service that can be unreachable in
  // some environments — degrade to an empty trend rather than failing the
  // whole page over a chart that isn't the page's only content. Pool status
  // isn't fetched here any more — BuyPanel fetches it itself client-side
  // (via /api/pool) since it needs live numbers after every swap, not just
  // on initial page load.
  const [stats, onchainHistory, viewsTrend, revenueTrend] = await Promise.all([
    fetchPlatformStats(),
    fetchPlatformMetricsHistory().catch(() => []),
    fetchPlatformViewsTrend(),
    fetchRevenueDistributedTrend().catch(() => []),
  ]);

  const scale = 10 ** config.solana.voteTokenDecimals;
  const appsTrend: TrendPoint[] = onchainHistory.map((p) => ({ x: p.capturedAt, y: p.appCount }));
  const tagsTrend: TrendPoint[] = onchainHistory.map((p) => ({ x: p.capturedAt, y: p.tagCount }));
  const votesTrend: TrendPoint[] = onchainHistory.map((p) => ({
    x: p.capturedAt,
    y: Number(p.totalVoteStake) / scale,
  }));
  const stakeTrend: TrendPoint[] = onchainHistory.map((p) => ({
    x: p.capturedAt,
    y: Number(p.totalTagStake) / scale,
  }));
  const viewsTrendPoints: TrendPoint[] = viewsTrend.map((p) => ({ x: p.date, y: p.totalViews }));
  const revenueTrendPoints: TrendPoint[] = revenueTrend.map((p) => ({ x: p.date, y: Number(p.amount) / scale }));
  const revenueTotal = Number(stats.totalRevenueDistributed) / scale;

  return (
    <div className="space-y-6">
      <PageHeader
        title="Rewards"
        description={
          <>
            Everything {TOKEN_SYMBOL} lives here: buy {TOKEN_NAME} on the public NEB/USDC pool,
            watch its live indicators, and claim what your votes and tag stakes have earned.
            Manage or unstake your positions from your profile page.
          </>
        }
      />

      <div className="space-y-6">
        <BuyPanel />
        <PlatformMetrics
          stats={stats}
          appsTrend={appsTrend}
          tagsTrend={tagsTrend}
          votesTrend={votesTrend}
          stakeTrend={stakeTrend}
          viewsTrend={viewsTrendPoints}
          revenueTrend={revenueTrendPoints}
          revenueTotal={revenueTotal}
          wide
        />
        <ClaimRewards />
        <CloseZeroStakeAccounts />
      </div>
    </div>
  );
}
