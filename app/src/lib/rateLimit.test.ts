import { describe, it, expect, beforeEach } from "vitest";
import { consumeRateLimit, _resetRateLimitsForTests } from "./rateLimit";

describe("consumeRateLimit", () => {
  beforeEach(() => {
    _resetRateLimitsForTests();
  });

  it("allows requests up to the limit within the window", () => {
    for (let i = 0; i < 3; i++) {
      const decision = consumeRateLimit("a", 3, 60_000, 1000);
      expect(decision.allowed).toBe(true);
    }
  });

  it("rejects the request once the limit is exceeded", () => {
    consumeRateLimit("a", 2, 60_000, 1000);
    consumeRateLimit("a", 2, 60_000, 1000);
    const decision = consumeRateLimit("a", 2, 60_000, 1000);
    expect(decision.allowed).toBe(false);
    expect(decision.remaining).toBe(0);
  });

  it("reports remaining requests correctly", () => {
    const first = consumeRateLimit("a", 5, 60_000, 1000);
    expect(first.remaining).toBe(4);
    const second = consumeRateLimit("a", 5, 60_000, 1000);
    expect(second.remaining).toBe(3);
  });

  it("resets the window after it expires", () => {
    consumeRateLimit("a", 1, 1000, 1000);
    const blocked = consumeRateLimit("a", 1, 1000, 1500);
    expect(blocked.allowed).toBe(false);
    const afterReset = consumeRateLimit("a", 1, 1000, 2100);
    expect(afterReset.allowed).toBe(true);
  });

  it("tracks separate keys independently", () => {
    consumeRateLimit("a", 1, 60_000, 1000);
    const otherKey = consumeRateLimit("b", 1, 60_000, 1000);
    expect(otherKey.allowed).toBe(true);
  });

  it("reports retryAfterMs when blocked", () => {
    consumeRateLimit("a", 1, 5000, 1000);
    const blocked = consumeRateLimit("a", 1, 5000, 2000);
    expect(blocked.retryAfterMs).toBe(4000);
  });
});
