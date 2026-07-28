import { test as base, expect } from "@playwright/test";

/**
 * Every test gets its own rate-limit identity, and no test is allowed to end
 * with an error alert on screen.
 *
 * POST /api/find/next calls requireRateLimit(req, RATE_LIMITS.read) — 60 per
 * 60_000 ms, keyed `read:ip:<clientIp>` (lib/api.ts). Under `next dev` there is
 * no proxy, so `x-forwarded-for` and `x-real-ip` are both absent and clientIp()
 * falls back to the literal string "unknown": every request this suite makes,
 * across every run, shares ONE fixed-window bucket. A suite run spends ~20 of
 * those 60, so the third or fourth back-to-back run of the documented command
 * tripped the limiter and the funnel rendered "Too many requests — try again in
 * Ns" where the spec expected the next question.
 *
 * Handing each test its own synthetic client address gives each its own
 * bucket. That is the precise fix: it isolates the tier from the limiter
 * without touching app/src, without a server restart per run, and without ever
 * waiting out a window — a tier that passes by sleeping still cannot tell a
 * mutant from a 429.
 *
 * COUPLING, see A101 in .agent/decisions.md. This depends on clientIp()'s
 * hop-selection rule in src/lib/api.ts, which now takes the LAST x-forwarded-for
 * hop (trusted-proxy depth of one). It works here only because `next dev` has NO
 * proxy in front, so the header below carries exactly one hop and first == last.
 * Put any proxy in front of the dev server and that value stops being the last
 * hop: every test collapses back onto one bucket and the suite starts failing on
 * the 3rd-4th consecutive run, looking exactly like fresh flakiness. The
 * afterEach below is the safety net — it names the 429 instead of letting it
 * masquerade as a missing element.
 */

// Unique per process, so back-to-back runs of the suite cannot collide either.
const RUN_OCTET = 1 + Math.floor(Math.random() * 254);
let testCounter = 0;

export const test = base.extend({
  extraHTTPHeaders: async ({}, use) => {
    testCounter += 1;
    await use({
      "x-forwarded-for": `10.${RUN_OCTET}.${(testCounter >> 8) & 255}.${testCounter & 255}`,
    });
  },
});

// A 429 reaches the UI as FindFunnel's error alert, and an alert on screen made
// every failure look like a generic "element not found". Failing on it by name
// means the limiter can never again be mistaken for a dead mutant — in either
// direction.
test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status === "failed" || testInfo.status === "timedOut") return;
  // Scoped to the funnel: `next dev` injects its own empty role="alert" into
  // the overlay portal, and Playwright pierces shadow DOM, so an unscoped
  // getByRole("alert") matches on every page.
  const alert = page.locator('[data-testid="find-funnel"] [role="alert"]');
  if ((await alert.count().catch(() => 0)) === 0) return;
  expect(await alert.first().innerText(), "funnel error alert was left on screen").toBe("");
});

export { expect };
