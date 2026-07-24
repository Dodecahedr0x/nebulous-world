import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { submitTxSchema } from "@/lib/validation";
import { submitSignedTx } from "@/lib/indexerClient";

// POST /api/tx/submit — relays an already wallet-signed transaction to the
// network via the indexer (sendRawTransaction + confirm). This is the only
// way any transaction (vote/stake/claim/buy) ever reaches the chain now —
// the app itself has no RPC connection to send one with directly. The most
// abuse-relevant endpoint here (arbitrary signed-tx broadcast), hence the
// tighter txSubmit limit rather than txBuild's.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txSubmit);
  const body = submitTxSchema.parse(await req.json());
  const result = await submitSignedTx(body.signedTransaction);
  return ok(result);
});
