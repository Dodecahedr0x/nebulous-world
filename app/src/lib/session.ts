import { createHmac, timingSafeEqual, randomBytes } from "crypto";
import { cookies } from "next/headers";
import { config } from "./config";

// Lightweight stateless session + auth-challenge handling.
//
// We avoid a session table by signing small payloads with HMAC-SHA256 using the
// server tracking secret. Both the auth challenge (nonce) and the resulting
// session cookie are self-describing and tamper-evident.

const SESSION_COOKIE = "nebulous_world_session";
const SESSION_TTL_MS = 7 * 24 * 60 * 60 * 1000; // 7 days
const CHALLENGE_TTL_MS = 5 * 60 * 1000; // 5 minutes

function sign(payload: string): string {
  return createHmac("sha256", config.tracking.secret)
    .update(payload)
    .digest("base64url");
}

function safeEqual(a: string, b: string): boolean {
  const ab = Buffer.from(a);
  const bb = Buffer.from(b);
  if (ab.length !== bb.length) return false;
  return timingSafeEqual(ab, bb);
}

// --- Auth challenge (stateless nonce) ---

/** Create a signed nonce that encodes its own issue time. */
export function createNonce(): string {
  const raw = `${Date.now()}.${randomBytes(16).toString("hex")}`;
  return `${raw}.${sign(raw)}`;
}

/** Verify a nonce is well-formed, correctly signed and not expired. */
export function verifyNonce(nonce: string): boolean {
  const parts = nonce.split(".");
  if (parts.length !== 3) return false;
  const [ts, rand, mac] = parts;
  const raw = `${ts}.${rand}`;
  if (!safeEqual(mac!, sign(raw))) return false;
  const issued = Number(ts);
  if (!Number.isFinite(issued)) return false;
  return Date.now() - issued <= CHALLENGE_TTL_MS;
}

// --- Session token ---

export interface SessionPayload {
  wallet: string;
  userId: string;
  exp: number;
}

export function issueSessionToken(wallet: string, userId: string): string {
  const payload: SessionPayload = {
    wallet,
    userId,
    exp: Date.now() + SESSION_TTL_MS,
  };
  const encoded = Buffer.from(JSON.stringify(payload)).toString("base64url");
  return `${encoded}.${sign(encoded)}`;
}

export function verifySessionToken(token: string): SessionPayload | null {
  const parts = token.split(".");
  if (parts.length !== 2) return null;
  const [encoded, mac] = parts;
  if (!safeEqual(mac!, sign(encoded!))) return null;
  try {
    const payload = JSON.parse(
      Buffer.from(encoded!, "base64url").toString("utf8"),
    ) as SessionPayload;
    if (typeof payload.exp !== "number" || Date.now() > payload.exp) return null;
    return payload;
  } catch {
    return null;
  }
}

// --- Cookie helpers (server components / route handlers) ---

export async function setSessionCookie(token: string): Promise<void> {
  const store = await cookies();
  store.set(SESSION_COOKIE, token, {
    httpOnly: true,
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
    path: "/",
    maxAge: SESSION_TTL_MS / 1000,
  });
}

export async function clearSessionCookie(): Promise<void> {
  const store = await cookies();
  store.delete(SESSION_COOKIE);
}

/** Read and verify the current session from cookies. Returns null if absent. */
export async function getSession(): Promise<SessionPayload | null> {
  const store = await cookies();
  const token = store.get(SESSION_COOKIE)?.value;
  if (!token) return null;
  return verifySessionToken(token);
}
