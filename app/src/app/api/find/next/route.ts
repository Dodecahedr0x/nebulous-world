import { NextRequest } from "next/server";
import { RATE_LIMITS, handler, ok, requireRateLimit } from "@/lib/api";
import { findNextSchema } from "@/lib/validation";
import { fetchNextFindQuestion } from "@/lib/indexerClient";
import { toFindNextInput } from "@/app/api/find/findRequest";

// POST /api/find/next — one turn of the /find funnel: the next question, or the
// final shortlist once the engine is confident.
//
// Stateless (A1): the client posts its whole answer history each turn and holds
// no server session. Writes nothing, so there is no Turnstile gate here.
export const POST = handler(async (req: NextRequest) => {
  // RATE_LIMITS.read (60/min) even though this endpoint writes nothing: every
  // call makes the indexer re-score the whole catalog, and it is the surface
  // an attempt to enumerate the catalog by sweeping answers would hammer.
  await requireRateLimit(req, RATE_LIMITS.read);

  // The funnel's first turn legitimately has an empty body, which the schema
  // defaults — a missing body must not be a 400.
  const body = findNextSchema.parse(await req.json().catch(() => ({})));

  const result = await fetchNextFindQuestion(toFindNextInput(body));
  return ok(result);
});
