import { NextRequest } from "next/server";
import { handler, ok, ApiError } from "@/lib/api";
import { fetchOpenGraph } from "@/lib/opengraph";

export const dynamic = "force-dynamic";

// GET /api/og?url=<page> — on-demand OpenGraph fetch for the create-app
// form's live card preview (see components/discover/CreateAppForm.tsx).
// Every other caller of lib/opengraph.ts's fetchOpenGraph runs offline
// (scripts/backfillOpengraph.ts, after an app already exists) — this is the
// only live path, hit once per debounced URL change while filling the form.
//
// Being the only live caller, this is also the one that pays for
// fetchOpenGraph's User-Agent fallback chain: a miss costs one attempt per
// entry in USER_AGENTS, so the worst case is now 2 x FETCH_TIMEOUT_MS (20s)
// rather than a single 5s timeout. In practice only a genuinely slow/hanging
// host reaches that — the common failure here is an immediate 403 from a
// bot-hostile site, where both attempts return in well under a second.
// Deliberately shares the chain with the offline path rather than running a
// cheaper single-UA fetch: this preview is supposed to show the user exactly
// the image that will end up on their card, and several sites only emit og:
// tags for the second (facebookexternalhit) attempt.
export const GET = handler(async (req: NextRequest) => {
  const url = req.nextUrl.searchParams.get("url");
  if (!url) throw new ApiError("url is required", 400);
  try {
    new URL(url);
  } catch {
    throw new ApiError("Invalid URL", 400);
  }

  const og = await fetchOpenGraph(url);
  return ok({ og });
});
