import { NextRequest } from "next/server";
import { handler, ok, ApiError, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { fetchVotePosition } from "@/lib/indexerClient";

export const dynamic = "force-dynamic";

// GET /api/accounts/vote-position/[appId]?owner=<pubkey> — decoded on-chain
// VotePosition for that owner, or `{ position: null }` if they've never
// voted on this app. `owner` is public data (a wallet address), so this is
// a plain query param rather than requiring an authenticated session.
export const GET = handler(
  async (req: NextRequest, ctx: { params: Promise<{ appId: string }> }) => {
    await requireRateLimit(req, RATE_LIMITS.read);
    const { appId } = await ctx.params;
    const owner = req.nextUrl.searchParams.get("owner");
    if (!owner) throw new ApiError("owner is required", 400);
    const position = await fetchVotePosition(appId, owner);
    return ok({ position });
  },
);
