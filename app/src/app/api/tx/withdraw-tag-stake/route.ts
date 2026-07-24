import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildStakeTagTxSchema } from "@/lib/validation";
import { buildWithdrawTagStakeTx } from "@/lib/indexerClient";

// POST /api/tx/withdraw-tag-stake — builds an unsigned `withdraw_tag_stake` transaction.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildStakeTagTxSchema.parse(await req.json());
  const built = await buildWithdrawTagStakeTx(body.appId, body.tagSlug, body.amount, body.user);
  return ok(built);
});
