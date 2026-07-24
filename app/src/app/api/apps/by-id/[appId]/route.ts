import { NextRequest } from "next/server";
import { handler, ok, ApiError } from "@/lib/api";
import { fetchAppById } from "@/lib/indexerClient";

export const dynamic = "force-dynamic";

// GET /api/apps/by-id/[appId] — polled by the client right after an
// on-chain app-creation transaction confirms, until the indexer has caught
// up and created the Postgres row (see indexer/src/processors/product.rs).
// Deliberately minimal (just enough to redirect to the real detail page) —
// full detail comes from GET /api/apps/[slug] once the caller has the slug.
export const GET = handler(
  async (_req: NextRequest, ctx: { params: Promise<{ appId: string }> }) => {
    const { appId } = await ctx.params;
    const app = await fetchAppById(appId);
    if (!app) throw new ApiError("Not indexed yet", 404);
    return ok(app);
  },
);
