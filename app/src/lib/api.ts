import { NextRequest, NextResponse } from "next/server";
import { ZodError } from "zod";
import { getSession, type SessionPayload } from "./session";
import { fetchUserById } from "./indexerClient";
import { consumeRateLimit } from "./rateLimit";

// Small helpers for consistent JSON API responses and auth guards.

export function ok<T>(data: T, init?: ResponseInit) {
  return NextResponse.json({ ok: true, data }, init);
}

export function fail(message: string, status = 400, extra?: unknown) {
  return NextResponse.json(
    { ok: false, error: message, details: extra },
    { status },
  );
}

/** Wrap a route handler so thrown errors become clean JSON responses. */
export function handler<T extends unknown[]>(
  fn: (...args: T) => Promise<Response>,
) {
  return async (...args: T): Promise<Response> => {
    try {
      return await fn(...args);
    } catch (err) {
      if (err instanceof ZodError) {
        return fail("Validation failed", 422, err.flatten());
      }
      if (err instanceof ApiError) {
        return fail(err.message, err.status);
      }
      console.error("[api] unhandled error:", err);
      return fail("Internal server error", 500);
    }
  };
}

export class ApiError extends Error {
  status: number;
  constructor(message: string, status = 400) {
    super(message);
    this.status = status;
  }
}

/** Require an authenticated session; throws 401 otherwise. */
export async function requireSession(): Promise<SessionPayload> {
  const session = await getSession();
  if (!session) throw new ApiError("Authentication required", 401);
  return session;
}

/** Require an authenticated session AND load the user record. */
export async function requireUser() {
  const session = await requireSession();
  const user = await fetchUserById(session.userId);
  if (!user) throw new ApiError("User not found", 401);
  return user;
}

// --- Rate limiting ---
//
// Every route under app/src/app/api/{tx,accounts,balances,wallet,pool}/**
// ultimately calls into the indexer, the app's only path to Solana RPC (see
// indexer/src/api.rs's doc comment) — none of it is otherwise authenticated
// or throttled, so it's the surface that needs protecting from abuse (a
// wallet or IP hammering tx-building/account-read endpoints). See
// lib/rateLimit.ts for the actual bucket algorithm.

/**
 * Requests allowed per window, by route category. `scope` keeps each
 * category's bucket independent per identity — without it, an anonymous
 * caller's `read` traffic would eat into their `auth` budget and vice versa,
 * since both would otherwise key on nothing but "this IP."
 */
export const RATE_LIMITS = {
  /** Builds an unsigned tx (one live RPC blockhash fetch per call). */
  txBuild: { scope: "txBuild", limit: 20, windowMs: 60_000 },
  /** Broadcasts an already-signed tx — the actual network-facing action. */
  txSubmit: { scope: "txSubmit", limit: 10, windowMs: 60_000 },
  /** Account/balance/pool reads — mostly served from the indexer's own DB mirror, not live RPC, so more headroom. */
  read: { scope: "read", limit: 60, windowMs: 60_000 },
  /** Auth challenge/verify — always IP-keyed (no session exists yet), tight enough to blunt brute-forcing a signature. */
  auth: { scope: "auth", limit: 10, windowMs: 60_000 },
} as const;

/**
 * The throttling identity for an unauthenticated caller.
 *
 * Takes the LAST `x-forwarded-for` entry, not the first. A reverse proxy
 * *appends* the peer it observed rather than replacing the header, so with
 * exactly one trusted proxy in front (Render's load balancer — `render.yaml`
 * declares `type: web`) the rightmost entry is the only value Render itself
 * vouched for. Everything to its left arrived from the caller and is therefore
 * attacker-chosen: reading index 0 let anyone mint a fresh rate-limit bucket
 * per request just by varying the header, which defeats every IP-keyed limit
 * in `RATE_LIMITS`.
 *
 * This is deliberately NOT the same rule as `tracking.ts`'s
 * `clientIpFromHeaders` — that answers "which visitor is this", where a
 * forged value costs the forger their own identity, while this answers "whom
 * do I throttle", where the field is adversarial by definition.
 *
 * Depth is one because Render is the only proxy. Putting another one in front
 * (Cloudflare proxying, a second LB) shifts the trusted entry left by one and
 * this must change with it — an unverified `x-forwarded-for` is never safe to
 * index from either end without knowing the depth.
 */
export function throttleIdentityFromHeaders(headers: Headers): string {
  const forwardedFor = headers.get("x-forwarded-for");
  if (forwardedFor) {
    const hops = forwardedFor
      .split(",")
      .map((hop) => hop.trim())
      .filter(Boolean);
    if (hops.length > 0) return hops[hops.length - 1]!;
  }
  // No proxy in front (local `next dev`, or direct-to-instance traffic).
  // `unknown` collapses every such caller into one shared bucket, which fails
  // closed: it throttles too much rather than not at all.
  return headers.get("x-real-ip") ?? "unknown";
}

function clientIp(req: NextRequest): string {
  return throttleIdentityFromHeaders(req.headers);
}

/**
 * Rate-limits the caller — by their verified session wallet (see
 * lib/solana-auth.ts's SIWS challenge/verify, the only thing that makes this
 * identity cost something to fake) when signed in, or by client IP
 * otherwise. Throws a 429 `ApiError` when the limit is exceeded; call this
 * first thing in a route handler, same as requireUser()/requireSession().
 */
export async function requireRateLimit(
  req: NextRequest,
  { scope, limit, windowMs }: { scope: string; limit: number; windowMs: number },
): Promise<void> {
  const session = await getSession();
  const identity = session ? `wallet:${session.wallet}` : `ip:${clientIp(req)}`;
  const decision = consumeRateLimit(`${scope}:${identity}`, limit, windowMs);
  if (!decision.allowed) {
    throw new ApiError(
      `Too many requests — try again in ${Math.ceil(decision.retryAfterMs / 1000)}s`,
      429,
    );
  }
}
