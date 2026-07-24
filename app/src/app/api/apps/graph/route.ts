import { NextRequest } from "next/server";
import { handler, ok } from "@/lib/api";
import { fetchAppGraph, mapRangeFiltersFromParams } from "@/lib/indexerClient";

export const dynamic = "force-dynamic";

// GET /api/apps/graph[?tags=slug1,slug2][&appStakeMin=...&pageviewsMax=...] —
// apps clustered by shared tags, for the Explore page's app map. `tags`, if
// given, restricts the map to apps carrying every one of those tags; the
// range params are the advanced-search filters (min/max app stake, min/max
// tag count, min/max pageviews) — see indexer/src/handlers/platform.rs's
// `build_app_graph`/`RangeQuery` doc comments.
export const GET = handler(async (req: NextRequest) => {
  const tags = req.nextUrl.searchParams.get("tags")?.split(",").filter(Boolean) ?? [];
  const ranges = mapRangeFiltersFromParams(req.nextUrl.searchParams);
  const graph = await fetchAppGraph(tags, ranges);
  return ok(graph);
});
