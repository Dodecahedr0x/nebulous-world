import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildVoteTxSchema } from "@/lib/validation";
import { buildVoteTx } from "@/lib/indexerClient";

// POST /api/tx/vote — builds an unsigned `vote` instruction transaction,
// proxied from the indexer. The client signs it with the connected wallet
// and posts the signed bytes to /api/tx/submit.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildVoteTxSchema.parse(await req.json());
  const built = await buildVoteTx(body.appId, body.amount, body.user);
  return ok(built);
});
