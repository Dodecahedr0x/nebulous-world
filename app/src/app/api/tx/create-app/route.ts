import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildCreateAppTxSchema } from "@/lib/validation";
import { buildCreateAppTx } from "@/lib/indexerClient";

// POST /api/tx/create-app — builds an unsigned transaction that creates the
// on-chain `AppAccount` (+ initial tags), proxied from the indexer. The
// client signs it with the connected wallet and posts the signed bytes to
// /api/tx/submit. There is no Prisma write here (or anywhere else in this
// route): the `App`/`Tag`/`AppTag` rows only exist once the indexer's
// crawler observes the confirmed transaction — see
// indexer/src/processors/product.rs.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildCreateAppTxSchema.parse(await req.json());
  const built = await buildCreateAppTx(body);
  return ok(built);
});
