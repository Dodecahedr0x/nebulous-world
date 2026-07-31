import { handler, ok } from "@/lib/api";
import { markDigestSeen } from "@/lib/indexerClient";
import { getSession } from "@/lib/session";

// POST /api/digest/seen — advances the digest watermark to now. Fired when
// the panel opens; the watermark only affects the NEXT load, so nothing
// disappears under the user's cursor. The userId comes from the session
// cookie, never from the request body — a caller can only ever advance
// their own watermark. Signed out is a no-op returning null, same
// convention as GET /api/digest.
export const POST = handler(async () => {
  const session = await getSession();
  if (!session) return ok(null);

  const result = await markDigestSeen(session.userId);
  return ok(result);
});
