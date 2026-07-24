import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { fetchAdsenseEarnings, getAdsenseAccessToken } from "./adsense";

describe("getAdsenseAccessToken", () => {
  beforeEach(() => {
    vi.stubEnv("ADSENSE_CLIENT_ID", "client-123");
    vi.stubEnv("ADSENSE_CLIENT_SECRET", "secret-abc");
    vi.stubEnv("ADSENSE_REFRESH_TOKEN", "refresh-xyz");
  });
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it("exchanges the refresh token for a fresh access token", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ access_token: "fresh-token", expires_in: 3600 }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const token = await getAdsenseAccessToken();

    expect(token).toBe("fresh-token");
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("https://oauth2.googleapis.com/token");
    const body = new URLSearchParams(init.body as string);
    expect(body.get("client_id")).toBe("client-123");
    expect(body.get("client_secret")).toBe("secret-abc");
    expect(body.get("refresh_token")).toBe("refresh-xyz");
    expect(body.get("grant_type")).toBe("refresh_token");
  });

  it("throws when the token endpoint responds with an error status", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: false, status: 400, text: async () => "invalid_grant" }),
    );
    await expect(getAdsenseAccessToken()).rejects.toThrow();
  });

  it("throws when any of the required env vars is missing", async () => {
    vi.stubEnv("ADSENSE_REFRESH_TOKEN", "");
    await expect(getAdsenseAccessToken()).rejects.toThrow(
      /ADSENSE_CLIENT_ID.*ADSENSE_CLIENT_SECRET.*ADSENSE_REFRESH_TOKEN/,
    );
  });
});

describe("fetchAdsenseEarnings", () => {
  beforeEach(() => vi.stubEnv("ADSENSE_ACCOUNT_ID", "pub-123"));
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it("returns the total earnings for the period from the AdSense API response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ totals: { cells: [{}, {}, { value: "142.37" }] } }),
      }),
    );
    const earnings = await fetchAdsenseEarnings(
      { start: new Date("2026-07-01"), end: new Date("2026-07-08") },
      "fake-access-token",
    );
    expect(earnings).toBeCloseTo(142.37, 2);
  });

  it("throws when the AdSense API responds with an error status", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: false, status: 401, text: async () => "unauthorized" }),
    );
    await expect(
      fetchAdsenseEarnings({ start: new Date("2026-07-01"), end: new Date("2026-07-08") }, "bad-token"),
    ).rejects.toThrow();
  });

  it("throws when ADSENSE_ACCOUNT_ID isn't configured", async () => {
    vi.stubEnv("ADSENSE_ACCOUNT_ID", "");
    await expect(
      fetchAdsenseEarnings({ start: new Date("2026-07-01"), end: new Date("2026-07-08") }, "token"),
    ).rejects.toThrow();
  });
});
