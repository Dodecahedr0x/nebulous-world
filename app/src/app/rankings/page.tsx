import { Suspense } from "react";
import type { Metadata } from "next";
import { SITE_URL } from "@/lib/constants";
import { PageHeader } from "@/components/PageHeader";
import { AutoRefresh } from "@/components/AutoRefresh";
import { PlatformMetrics } from "@/components/rewards/PlatformMetrics";
import { Leaderboard } from "@/components/rankings/Leaderboard";
import { RankingsTabs } from "@/components/rankings/RankingsTabs";
import { ExploreMaps } from "@/components/explore/ExploreMaps";
import {
  fetchPlatformStats,
  fetchPlatformMetricsHistory,
  fetchPlatformViewsTrend,
  searchApps,
} from "@/lib/indexerClient";
import { searchSchema } from "@/lib/validation";
import { config } from "@/lib/config";
import type { TrendPoint } from "@/components/explore/MetricTrendCard";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Rankings",
  description: "See how nebulous.world ranks apps — a live leaderboard plus a map of how apps and tags connect.",
  alternates: { canonical: `${SITE_URL}/rankings` },
};

const LEADERBOARD_SIZE = 50;

export default async function RankingsPage() {
  // Same degrade-gracefully reasoning as rewards/page.tsx: the on-chain-
  // derived history comes from the indexer, a separate service that can be
  // unreachable in some environments — an empty trend shouldn't fail the
  // whole page over a chart that isn't the page's only content.
  const [stats, onchainHistory, viewsTrend, searchResult] = await Promise.all([
    fetchPlatformStats(),
    fetchPlatformMetricsHistory().catch(() => []),
    fetchPlatformViewsTrend(),
    searchApps(searchSchema.parse({ sort: "rank", pageSize: LEADERBOARD_SIZE })),
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

  return (
    <div className="space-y-6">
      <AutoRefresh />
      <PageHeader
        title="Rankings"
        description="A live leaderboard of every app on nebulous.world, plus a map of how apps and tags connect."
      />

      <PlatformMetrics
        stats={stats}
        appsTrend={appsTrend}
        tagsTrend={tagsTrend}
        votesTrend={votesTrend}
        stakeTrend={stakeTrend}
        viewsTrend={viewsTrendPoints}
      />

      <Suspense fallback={<div className="py-16 text-center text-slate-steel">Loading…</div>}>
        <RankingsTabs
          leaderboard={<Leaderboard apps={searchResult.apps.slice(0, LEADERBOARD_SIZE)} />}
          map={<ExploreMaps />}
        />
      </Suspense>
    </div>
  );
}
