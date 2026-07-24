import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { fetchCloseablePositions } from "@/lib/indexerClient";

export const dynamic = "force-dynamic";

// GET /api/wallet/[owner]/closeable-positions — every VotePosition/StakePosition
// `owner` currently holds at zero stake, closeable to reclaim rent. `owner`
// is public data (a wallet address), so no auth is required, same as the
// other /api/accounts/** reads.
export const GET = handler(
  async (req: NextRequest, ctx: { params: Promise<{ owner: string }> }) => {
    await requireRateLimit(req, RATE_LIMITS.read);
    const { owner } = await ctx.params;
    const positions = await fetchCloseablePositions(owner);
    return ok({ positions });
  },
);
