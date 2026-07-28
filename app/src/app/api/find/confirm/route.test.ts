import { NextRequest } from "next/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The route itself, not just its helpers. findRequest.ts's header comment
// explains why route files were previously left untested — `requireRateLimit`
// reaches for next/headers' `cookies()`, which throws outside a request scope —
// so that one function is replaced here and everything else runs for real. The
// guard this file exists for is A80: a failed Turnstile must no longer refuse
// the write, and nothing but an end-to-end read of the route can show that the
// 403 is gone AND that the flag reaching the indexer is right.

const requireRateLimit = vi.fn<[unknown, unknown], Promise<void>>();
const recordFindOutcome = vi.fn(async () => ({ ok: true as const }));
const verifyTurnstileToken = vi.fn<[string | null], Promise<boolean>>();
const resolveVisitor = vi.fn(() => ({
  visitorId: "server-visitor",
  sessionId: "server-session",
  userAgent: "Mozilla/5.0",
  isBot: false,
}));

vi.mock("@/lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api")>()),
  requireRateLimit: (...args: unknown[]) => requireRateLimit(args[0], args[1]),
}));
vi.mock("@/lib/indexerClient", () => ({ recordFindOutcome }));
vi.mock("@/lib/turnstile", () => ({ verifyTurnstileToken }));
vi.mock("@/lib/tracking", () => ({ resolveVisitor }));

const { POST } = await import("./route");
const { RATE_LIMITS } = await import("@/lib/api");

function post(body: unknown) {
  return POST(
    new NextRequest("http://localhost/api/find/confirm", {
      method: "POST",
      headers: { "content-type": "application/json", "user-agent": "Mozilla/5.0" },
      body: JSON.stringify(body),
    }),
  );
}

const CONFIRM = {
  answers: [{ facet: { kind: "tag", value: "lending" }, value: "yes" }],
  appId: "app-1",
  outcome: "confirmed",
  turnstileToken: "tok",
};

/** The single argument `recordFindOutcome` was called with. */
function forwarded() {
  expect(recordFindOutcome).toHaveBeenCalledTimes(1);
  return recordFindOutcome.mock.calls[0][0] as Record<string, unknown>;
}

const ORIGINAL_SECRET = process.env.TURNSTILE_SECRET_KEY;

beforeEach(() => {
  vi.clearAllMocks();
  requireRateLimit.mockResolvedValue(undefined);
  recordFindOutcome.mockResolvedValue({ ok: true });
  resolveVisitor.mockReturnValue({
    visitorId: "server-visitor",
    sessionId: "server-session",
    userAgent: "Mozilla/5.0",
    isBot: false,
  });
});

afterEach(() => {
  if (ORIGINAL_SECRET === undefined) delete process.env.TURNSTILE_SECRET_KEY;
  else process.env.TURNSTILE_SECRET_KEY = ORIGINAL_SECRET;
});

describe("POST /api/find/confirm", () => {
  /**
   * A80, the whole point of this change. Under A29 this was a 403 and the row
   * was dropped — and Turnstile fails hardest for VPN users, privacy browsers
   * and slow connections, so what was dropped was a biased slice, not a random
   * one, and its size was unmeasurable because nothing recorded the attempt.
   */
  it("records the outcome as unverified instead of 403ing when the token fails", async () => {
    process.env.TURNSTILE_SECRET_KEY = "secret";
    verifyTurnstileToken.mockResolvedValue(false);

    const res = await post(CONFIRM);

    expect(res.status).toBe(200);
    expect(forwarded().turnstileVerified).toBe(false);
  });

  it("marks a passing token verified", async () => {
    process.env.TURNSTILE_SECRET_KEY = "secret";
    verifyTurnstileToken.mockResolvedValue(true);

    await post(CONFIRM);

    expect(forwarded().turnstileVerified).toBe(true);
  });

  /**
   * A4: the funnel must work with no infrastructure. `verifyTurnstileToken`
   * returns false for an unconfigured environment exactly as it does for a bad
   * token, so without the env-var check every local and simulation outcome
   * would be written unverified and `load_learned` would train on nothing.
   */
  it("counts an unconfigured environment as verified so local runs still train", async () => {
    delete process.env.TURNSTILE_SECRET_KEY;
    verifyTurnstileToken.mockResolvedValue(false);

    await post({ ...CONFIRM, turnstileToken: null });

    expect(forwarded().turnstileVerified).toBe(true);
  });

  /** A28: a client-chosen identity would make the indexer's idempotency key
      attacker-chosen, i.e. free confirmation farming. */
  it("takes visitorId and sessionId from the server, never the body", async () => {
    process.env.TURNSTILE_SECRET_KEY = "secret";
    verifyTurnstileToken.mockResolvedValue(true);

    await post({ ...CONFIRM, visitorId: "attacker", sessionId: "attacker" });

    const input = forwarded();
    expect(input.visitorId).toBe("server-visitor");
    expect(input.sessionId).toBe("server-session");
  });

  /** A20: the confirm response is `{ok:true}` and gains nothing. Leaking the
      verification result would tell a farmer which tokens are working. */
  it("does not disclose the verification result", async () => {
    process.env.TURNSTILE_SECRET_KEY = "secret";
    verifyTurnstileToken.mockResolvedValue(false);

    const body = await (await post(CONFIRM)).json();

    expect(body).toEqual({ ok: true, data: { ok: true } });
    expect(JSON.stringify(body)).not.toContain("turnstile");
  });

  /** The token is a credential for Cloudflare, not payload — it must not ride
      along to the indexer, which has no use for it and would then log it. */
  it("never forwards the raw turnstile token", async () => {
    process.env.TURNSTILE_SECRET_KEY = "secret";
    verifyTurnstileToken.mockResolvedValue(true);

    await post(CONFIRM);

    expect(forwarded()).not.toHaveProperty("turnstileToken");
  });

  /** Rate limiting is the gate that survived A80; dropping it while removing
      the 403 would leave the write path with no volume cost at all. */
  it("rate-limits before doing anything else", async () => {
    process.env.TURNSTILE_SECRET_KEY = "secret";
    requireRateLimit.mockRejectedValue(
      new (await import("@/lib/api")).ApiError("Too many requests", 429),
    );

    const res = await post(CONFIRM);

    expect(res.status).toBe(429);
    expect(requireRateLimit).toHaveBeenCalledWith(expect.anything(), RATE_LIMITS.auth);
    expect(recordFindOutcome).not.toHaveBeenCalled();
  });

  /** Matching /api/track: a bot learns nothing from the response, and a 4xx is
      information. The row is not written, so it cannot pollute the flag either. */
  it("silently ignores bots without recording", async () => {
    resolveVisitor.mockReturnValue({
      visitorId: "bot",
      sessionId: "bot",
      userAgent: "Googlebot",
      isBot: true,
    });

    const res = await post(CONFIRM);

    expect(res.status).toBe(200);
    expect(recordFindOutcome).not.toHaveBeenCalled();
  });
});
