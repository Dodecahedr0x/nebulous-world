import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildClosePositionTxSchema } from "@/lib/validation";
import { buildCloseVotePositionTx } from "@/lib/indexerClient";

// POST /api/tx/close-vote-position — builds an unsigned `close_vote_position` transaction.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildClosePositionTxSchema.parse(await req.json());
  const built = await buildCloseVotePositionTx(body.position, body.user);
  return ok(built);
});
