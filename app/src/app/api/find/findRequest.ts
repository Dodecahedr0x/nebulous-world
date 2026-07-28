import type { FindAnswer, FindConfirmInput, FindNextInput } from "@/lib/types";
import type { FindConfirmRequest, FindNextRequest } from "@/lib/validation";

// Request-shaping for the two /find routes, kept out of the route files so it
// is unit-testable: `requireRateLimit` reaches for next/headers' `cookies()`,
// which throws outside a request scope, so importing a route.ts into Vitest is
// not viable.

/** Cap enforced before the body reaches the indexer, matching findNextSchema's
    own `.max(16)`. Each answer costs the engine a full re-score of the
    catalog, so this bounds that work independently of Zod. */
export const MAX_FORWARDED_ANSWERS = 16;

function facetKey(answer: FindAnswer): string {
  return `${answer.facet.kind}:${answer.facet.value}`;
}

/**
 * The last answer for each facet wins, and order is preserved by first
 * appearance — the client's back/re-answer flow re-sends its whole history
 * (the funnel is stateless per A1), so a facet can legitimately appear twice
 * with different values, and the engine must score each facet at most once.
 */
export function dedupeAnswers(answers: FindAnswer[]): FindAnswer[] {
  const byFacet = new Map<string, FindAnswer>();
  for (const answer of answers) {
    byFacet.set(facetKey(answer), answer);
  }
  return [...byFacet.values()];
}

/** Body -> indexer payload for POST /find/next. */
export function toFindNextInput(body: FindNextRequest): FindNextInput {
  return {
    answers: dedupeAnswers(body.answers).slice(0, MAX_FORWARDED_ANSWERS),
    forceResults: body.forceResults,
  };
}

/**
 * Body + server-derived visitor identity -> indexer payload for
 * POST /find/confirm. `visitorId`/`sessionId` come from resolveVisitor, never
 * from the request body (A28) — a client-chosen identity would make the
 * (sessionId, appId, outcome) idempotency key attacker-chosen. Fields are
 * listed explicitly rather than spread so `turnstileToken` cannot ride along.
 */
export function toFindConfirmInput(
  body: FindConfirmRequest,
  visitor: { visitorId: string; sessionId: string },
): FindConfirmInput {
  return {
    answers: dedupeAnswers(body.answers).slice(0, MAX_FORWARDED_ANSWERS),
    appId: body.appId,
    outcome: body.outcome,
    visitorId: visitor.visitorId,
    sessionId: visitor.sessionId,
  };
}

/**
 * Turnstile gating (A29). `verified` is verifyTurnstileToken's result;
 * `configured` is Boolean(process.env.TURNSTILE_SECRET_KEY), read by the
 * caller. Returns true when the outcome may be recorded.
 *
 * verifyTurnstileToken returns false both for a failed token *and* for an
 * unconfigured environment, so gating on it unconditionally would break /find
 * entirely in local and simulation mode — which contradicts the requirement
 * that the funnel work with no wallet and no infrastructure (A4). Reading the
 * env var is the only way to tell the two cases apart.
 */
export function mayRecordOutcome(configured: boolean, verified: boolean): boolean {
  return !configured || verified;
}
