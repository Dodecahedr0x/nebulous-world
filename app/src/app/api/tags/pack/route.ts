import { NextRequest } from "next/server";
import { handler, ok } from "@/lib/api";
import { fetchTagPack, mapRangeFiltersFromParams } from "@/lib/indexerClient";

export const dynamic = "force-dynamic";

// GET /api/tags/pack[?appStakeMin=...&pageviewsMax=...] — every approved
// app's full tag list plus each tag's global popularity, for the Explore
// page's Group (circle-packing) tab. The range params are the same
// advanced-search filters /api/apps/graph takes.
export const GET = handler(async (req: NextRequest) => {
  const ranges = mapRangeFiltersFromParams(req.nextUrl.searchParams);
  const pack = await fetchTagPack(ranges);
  return ok(pack);
});
