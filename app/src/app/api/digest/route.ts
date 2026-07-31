import { handler, ok } from "@/lib/api";
import { fetchDigest } from "@/lib/indexerClient";
import { getSession } from "@/lib/session";

// GET /api/digest — the signed-in user's "since you were last here" digest,
// or null if signed out. Same signed-out-returns-null convention as
// /api/xp/me: there is no anonymous digest, and the bell simply isn't
// rendered in that case (see docs/plans/2026-08-01-staker-digest-design.md).
export const GET = handler(async () => {
  const session = await getSession();
  if (!session) return ok(null);

  const digest = await fetchDigest(session.userId);
  return ok(digest);
});
