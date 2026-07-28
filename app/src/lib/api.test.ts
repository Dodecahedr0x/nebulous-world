import { describe, expect, it } from "vitest";
import { throttleIdentityFromHeaders } from "@/lib/api";

const h = (init: Record<string, string>) => new Headers(init);

describe("throttleIdentityFromHeaders", () => {
  // The whole point of the function. A reverse proxy APPENDS the peer it saw
  // rather than replacing the header, so with Render as the single trusted
  // proxy the rightmost entry is the only one Render vouched for. Reading
  // index 0 returns whatever the caller sent, which is what made every
  // IP-keyed limit in RATE_LIMITS evadable.
  it("takes the last hop, because everything left of it is caller-supplied", () => {
    expect(throttleIdentityFromHeaders(h({ "x-forwarded-for": "203.0.113.9" }))).toBe("203.0.113.9");
    expect(
      throttleIdentityFromHeaders(h({ "x-forwarded-for": "spoofed, 203.0.113.9" })),
    ).toBe("203.0.113.9");
  });

  // The attack this closes: a caller varying the header per request used to
  // mint a fresh bucket each time. Every one of these must resolve to the SAME
  // identity, since Render appends the same real peer to each.
  it("gives a spoofing caller one identity, not one per request", () => {
    const spoofs = ["a", "b", "c", "10.0.0.1", ""].map((fake) =>
      throttleIdentityFromHeaders(h({ "x-forwarded-for": `${fake}, 203.0.113.9` })),
    );
    expect(new Set(spoofs).size).toBe(1);
    expect(spoofs[0]).toBe("203.0.113.9");
  });

  it("handles whitespace and empty hops without yielding an empty bucket key", () => {
    expect(throttleIdentityFromHeaders(h({ "x-forwarded-for": "  spoofed ,  203.0.113.9  " }))).toBe(
      "203.0.113.9",
    );
    expect(throttleIdentityFromHeaders(h({ "x-forwarded-for": "203.0.113.9, ," }))).toBe(
      "203.0.113.9",
    );
    expect(throttleIdentityFromHeaders(h({ "x-forwarded-for": " , " }))).toBe("unknown");
  });

  it("falls back to x-real-ip, then to a shared bucket that fails closed", () => {
    expect(throttleIdentityFromHeaders(h({ "x-real-ip": "198.51.100.4" }))).toBe("198.51.100.4");
    expect(throttleIdentityFromHeaders(h({}))).toBe("unknown");
  });
});
