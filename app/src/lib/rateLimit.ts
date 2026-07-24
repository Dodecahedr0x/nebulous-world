// Fixed-window rate limiting, keyed by an arbitrary string (a verified
// wallet or a client IP — see lib/api.ts's requireRateLimit). Pure and
// framework-agnostic so it's directly unit-testable; the Next.js-specific
// bits (reading the session/IP, throwing a 429) live in lib/api.ts.
//
// In-memory only: state lives in this module-level Map, which resets on
// every deploy/restart and isn't shared across horizontal instances. That's
// an acceptable trade-off at this app's current single-instance scale (see
// render.yaml) — swap for a shared store (Redis/Upstash) before scaling
// out to more than one instance, since each instance would otherwise track
// its own independent counters and the effective limit would multiply by
// instance count.

interface Bucket {
  count: number;
  resetAt: number;
}

const buckets = new Map<string, Bucket>();

// Sweep expired buckets periodically rather than on every call, so a busy
// endpoint doesn't pay for a full Map scan on every request — this only
// bounds memory growth from the (unbounded) variety of keys seen over time,
// it's not part of the rate-limit decision itself.
const SWEEP_EVERY_N_CALLS = 500;
let callsSinceSweep = 0;

function sweepExpired(now: number): void {
  callsSinceSweep += 1;
  if (callsSinceSweep < SWEEP_EVERY_N_CALLS) return;
  callsSinceSweep = 0;
  for (const [key, bucket] of buckets) {
    if (now >= bucket.resetAt) buckets.delete(key);
  }
}

export interface RateLimitDecision {
  allowed: boolean;
  /** Requests left in the current window if allowed; 0 if not. */
  remaining: number;
  /** How long until the window resets, in ms — 0 if allowed. */
  retryAfterMs: number;
}

/**
 * Consumes one request against `key`'s bucket, allowing up to `limit`
 * requests per `windowMs`. Fixed windows (not sliding/token-bucket) — simple
 * and enough to blunt abuse; a client can burst up to `limit` right at a
 * window boundary, which is an acceptable trade-off for the complexity it
 * avoids.
 */
export function consumeRateLimit(
  key: string,
  limit: number,
  windowMs: number,
  now: number = Date.now(),
): RateLimitDecision {
  sweepExpired(now);

  const existing = buckets.get(key);
  if (!existing || now >= existing.resetAt) {
    buckets.set(key, { count: 1, resetAt: now + windowMs });
    return { allowed: true, remaining: limit - 1, retryAfterMs: 0 };
  }
  if (existing.count >= limit) {
    return { allowed: false, remaining: 0, retryAfterMs: existing.resetAt - now };
  }
  existing.count += 1;
  return { allowed: true, remaining: limit - existing.count, retryAfterMs: 0 };
}

/** Test-only: clears all bucket state so tests don't leak between each other. */
export function _resetRateLimitsForTests(): void {
  buckets.clear();
  callsSinceSweep = 0;
}
