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

function clientIp(req: NextRequest): string {
  // Render (like most PaaS reverse proxies) sets x-forwarded-for; take the
  // first hop, which is the original client.
  const forwardedFor = req.headers.get("x-forwarded-for");
  if (forwardedFor) return forwardedFor.split(",")[0]!.trim();
  return req.headers.get("x-real-ip") ?? "unknown";
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
