import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { fetchAppAccount } from "@/lib/indexerClient";

export const dynamic = "force-dynamic";

// GET /api/accounts/app/[appId] — decoded on-chain AppAccount (vault
// addresses, reward accumulators), proxied from the indexer instead of an
// RPC `program.account.appAccount.fetch()` call.
export const GET = handler(
  async (req: NextRequest, ctx: { params: Promise<{ appId: string }> }) => {
    await requireRateLimit(req, RATE_LIMITS.read);
    const { appId } = await ctx.params;
    const app = await fetchAppAccount(appId);
    return ok({ app });
  },
);
