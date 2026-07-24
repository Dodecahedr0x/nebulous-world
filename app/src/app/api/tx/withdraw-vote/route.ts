import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildVoteTxSchema } from "@/lib/validation";
import { buildWithdrawVoteTx } from "@/lib/indexerClient";

// POST /api/tx/withdraw-vote — builds an unsigned `withdraw_vote` transaction.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildVoteTxSchema.parse(await req.json());
  const built = await buildWithdrawVoteTx(body.appId, body.amount, body.user);
  return ok(built);
});
