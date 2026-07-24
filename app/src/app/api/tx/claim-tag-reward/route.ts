import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildClaimTagRewardTxSchema } from "@/lib/validation";
import { buildClaimTagRewardTx } from "@/lib/indexerClient";

// POST /api/tx/claim-tag-reward — builds an unsigned `claim_tag_reward` transaction.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildClaimTagRewardTxSchema.parse(await req.json());
  const built = await buildClaimTagRewardTx(body.appId, body.tagSlug, body.user);
  return ok(built);
});
