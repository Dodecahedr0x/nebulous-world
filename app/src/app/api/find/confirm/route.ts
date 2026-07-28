import { NextRequest } from "next/server";
import { RATE_LIMITS, handler, ok, requireRateLimit } from "@/lib/api";
import { findConfirmSchema } from "@/lib/validation";
import { recordFindOutcome } from "@/lib/indexerClient";
import { resolveVisitor } from "@/lib/tracking";
import { verifyTurnstileToken } from "@/lib/turnstile";
import { mayRecordOutcome, toFindConfirmInput } from "@/app/api/find/findRequest";
import type { FindConfirmInput } from "@/lib/types";

// POST /api/find/confirm — record how a /find session ended (confirmed,
// rejected, or clicked through), which is the funnel's training signal.
//
// Surfacing in /find drives traffic and therefore staker revenue, so farming
// confirmations is directly profitable. The defence is a bounded-influence
// blend, plus Turnstile and rate limiting: the real bound lives in the
// indexer's find/blend.rs, which caps how far the learned term can move a
// ranking at any support level. These two gates only raise the cost of volume.
export const POST = handler(async (req: NextRequest) => {
  // RATE_LIMITS.auth (10/min) rather than a new bucket — @/lib/api is outside
  // this node's scope, and auth's tight window is the right shape for a write
  // path that is cheap to forge. Deliberate reuse, not a copy-paste slip (A30).
  await requireRateLimit(req, RATE_LIMITS.auth);

  const body = findConfirmSchema.parse(await req.json());

  // Server-derived, never from the body (A28): a client-chosen identity would
  // make the indexer's (sessionId, appId, outcome) idempotency key
  // attacker-chosen, which is free confirmation farming.
  const visitor = resolveVisitor(req.headers);
  // Silently ignored rather than refused, matching /api/track: a bot should
  // learn nothing from the response, and a 4xx is information.
  if (visitor.isBot) return ok({ ok: true });

  // A80 supersedes A29: a failed or timed-out token no longer refuses the
  // write. Turnstile fails hardest for VPN users, privacy browsers and slow
  // connections, so a 403 here dropped a *biased* slice of outcomes and made
  // the size of that slice unmeasurable at the same time — the A61/A62 defect
  // again, one layer down. The row is recorded either way and carries whether
  // it was verified; the indexer's store::load_learned trains only on the
  // verified ones, so farming still buys nothing.
  //
  // `mayRecordOutcome` is reused, not reimplemented: its rule is unchanged
  // (unconfigured environments count as verified, since verifyTurnstileToken
  // cannot distinguish "no secret key" from "bad token"), only its consequence
  // moved from gate to flag. Its name now understates what it returns, and
  // findRequest.ts is outside this fix's scope to rename.
  const configured = Boolean(process.env.TURNSTILE_SECRET_KEY);
  const verified = await verifyTurnstileToken(body.turnstileToken ?? null);
  const turnstileVerified = mayRecordOutcome(configured, verified);

  // `FindConfirmInput` does not carry the flag and lib/types.ts is out of
  // scope here, so it is attached at the boundary and widened locally.
  const input: FindConfirmInput & { turnstileVerified: boolean } = {
    ...toFindConfirmInput(body, visitor),
    turnstileVerified,
  };

  const result = await recordFindOutcome(input);
  return ok(result);
});
