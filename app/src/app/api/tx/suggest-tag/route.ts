import { NextRequest } from "next/server";
import { handler, ok, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { buildSuggestTagTxSchema } from "@/lib/validation";
import { buildSuggestTagTx } from "@/lib/indexerClient";

// POST /api/tx/suggest-tag — builds an unsigned `suggest_tag` transaction
// for an app that already exists, proxied from the indexer. Same
// no-Prisma-write shape as /api/tx/create-app: the `Tag`/`AppTag` rows show
// up once the indexer observes the confirmed transaction.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.txBuild);
  const body = buildSuggestTagTxSchema.parse(await req.json());
  const built = await buildSuggestTagTx(body.appId, body.tagSlug, body.user);
  return ok(built);
});
