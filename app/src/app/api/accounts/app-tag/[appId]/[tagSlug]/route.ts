import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { fetchAppTagStake } from "@/lib/indexerClient";

export const dynamic = "force-dynamic";

// GET /api/accounts/app-tag/[appId]/[tagSlug] — decoded on-chain
// AppTagStake (the (app, tag) stake-accounting connection; the tag identity
// itself is a separate, global on-chain Tag account), proxied from the
// indexer.
export const GET = handler(
  async (req: NextRequest, ctx: { params: Promise<{ appId: string; tagSlug: string }> }) => {
    await requireRateLimit(req, RATE_LIMITS.read);
    const { appId, tagSlug } = await ctx.params;
    const appTag = await fetchAppTagStake(appId, tagSlug);
    return ok({ appTag });
  },
);
