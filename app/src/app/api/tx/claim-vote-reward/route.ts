import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildClaimVoteRewardTxSchema } from "@/lib/validation";
import { buildClaimVoteRewardTx } from "@/lib/indexerClient";

// POST /api/tx/claim-vote-reward — builds an unsigned `claim_vote_reward` transaction.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildClaimVoteRewardTxSchema.parse(await req.json());
  const built = await buildClaimVoteRewardTx(body.appId, body.user);
  return ok(built);
});
