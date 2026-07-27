import { describe, it, expect, vi, afterEach } from "vitest";
import { fetchOpenGraph, enrichWithOpenGraph } from "./opengraph";

function htmlResponse(html: string, opts: Partial<{ ok: boolean; status: number; contentType: string; url: string }> = {}) {
  const body = new TextEncoder().encode(html);
  return {
    ok: opts.ok ?? true,
    status: opts.status ?? 200,
    url: opts.url ?? "https://example.com/",
    headers: new Headers({ "content-type": opts.contentType ?? "text/html; charset=utf-8" }),
    body: {
      getReader() {
        let sent = false;
        return {
          async read() {
            if (sent) return { done: true, value: undefined };
            sent = true;
            return { done: false, value: body };
          },
          async cancel() {},
        };
      },
    },
    async text() {
      return html;
    },
  } as unknown as Response;
}

describe("fetchOpenGraph", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("extracts og:image, og:title, og:description", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<html><head>
          <meta property="og:title" content="Jupiter" />
          <meta property="og:description" content="Solana's liquidity aggregator" />
          <meta property="og:image" content="https://jup.ag/og.png" />
        </head></html>`),
      ),
    );
    const data = await fetchOpenGraph("https://jup.ag");
    expect(data).toEqual({
      title: "Jupiter",
      description: "Solana's liquidity aggregator",
      imageUrl: "https://jup.ag/og.png",
    });
  });

  it("doesn't let a preceding meta tag's content bleed into a later og:image capture (real-world minified markup, no whitespace between tags)", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(
          `<meta content="A generic description." name="description"/>` +
            `<meta content="Some Title" property="og:title"/>` +
            `<meta content="A generic description." property="og:description"/>` +
            `<meta content="https://cdn.example.com/real-og-image.png" property="og:image"/>`,
        ),
      ),
    );
    const data = await fetchOpenGraph("https://example.com");
    expect(data?.imageUrl).toBe("https://cdn.example.com/real-og-image.png");
  });

  it("resolves a relative og:image against the response URL", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<meta property="og:image" content="/static/og.png" />`, {
          url: "https://example.com/app/",
        }),
      ),
    );
    const data = await fetchOpenGraph("https://example.com/app");
    expect(data?.imageUrl).toBe("https://example.com/static/og.png");
  });

  it("falls back to twitter:image when og:image is absent", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<meta name="twitter:image" content="https://example.com/twitter.png" />`),
      ),
    );
    const data = await fetchOpenGraph("https://example.com");
    expect(data?.imageUrl).toBe("https://example.com/twitter.png");
  });

  it("handles content before property in the meta tag", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<meta content="https://example.com/og.png" property="og:image" />`),
      ),
    );
    const data = await fetchOpenGraph("https://example.com");
    expect(data?.imageUrl).toBe("https://example.com/og.png");
  });

  it("decodes HTML entities in title/description", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<meta property="og:title" content="Fish &amp; Chips" />`),
      ),
    );
    const data = await fetchOpenGraph("https://example.com");
    expect(data?.title).toBe("Fish & Chips");
  });

  it("returns null when no OpenGraph tags are present", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(htmlResponse(`<html><head></head></html>`)));
    const data = await fetchOpenGraph("https://example.com");
    expect(data).toBeNull();
  });

  it("returns null on a non-ok response", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(htmlResponse("", { ok: false, status: 404 })));
    const data = await fetchOpenGraph("https://example.com/missing");
    expect(data).toBeNull();
  });

  it("returns null for a non-HTML response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(htmlResponse("{}", { contentType: "application/json" })),
    );
    const data = await fetchOpenGraph("https://example.com/api");
    expect(data).toBeNull();
  });

  it("returns null when fetch rejects (network error)", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("network down")));
    const data = await fetchOpenGraph("https://unreachable.example.com");
    expect(data).toBeNull();
  });

  it("returns null when the fetch is aborted (timeout)", async () => {
    vi.useFakeTimers();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((_url: string, init: RequestInit) => {
        return new Promise((_resolve, reject) => {
          init.signal?.addEventListener("abort", () => {
            const err = new Error("aborted");
            err.name = "AbortError";
            reject(err);
          });
        });
      }),
    );
    const resultPromise = fetchOpenGraph("https://slow.example.com");
    await vi.runAllTimersAsync();
    const data = await resultPromise;
    expect(data).toBeNull();
    vi.useRealTimers();
  });

  // The fallback chain below exists because a majority of real sites serve no
  // og: tags at all to a non-JS client — see the counts in opengraph.ts.
  it("falls back to <link rel=apple-touch-icon> when there is no og:image", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<head>
          <title>Strava</title>
          <link rel="icon" href="/small.ico">
          <link rel="apple-touch-icon" href="/apple-touch-icon.png">
        </head>`),
      ),
    );
    const data = await fetchOpenGraph("https://www.strava.com");
    expect(data?.imageUrl).toBe("https://example.com/apple-touch-icon.png");
    expect(data?.title).toBe("Strava");
  });

  it("uses <title> and <meta name=description> when no og:/twitter: tags exist", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<head><title>Electrum</title>
          <meta name="description" content="Bitcoin wallet"></head>`),
      ),
    );
    const data = await fetchOpenGraph("https://electrum.org");
    expect(data?.title).toBe("Electrum");
    expect(data?.description).toBe("Bitcoin wallet");
  });

  it("never lets a <link> icon displace a real og:image", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`<head>
          <link rel="apple-touch-icon" href="/icon.png">
          <meta property="og:image" content="https://cdn.example.com/card.png">
        </head>`),
      ),
    );
    const data = await fetchOpenGraph("https://example.com");
    expect(data?.imageUrl).toBe("https://cdn.example.com/card.png");
  });

  // The only source that survives a site refusing the page fetch outright.
  it("probes well-known icon paths when every page attempt is refused", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(async (input: string | URL) => {
        const url = String(input);
        if (url.endsWith("/apple-touch-icon.png")) {
          return {
            ok: true,
            url,
            headers: new Headers({ "content-type": "image/png" }),
          } as unknown as Response;
        }
        return { ok: false, status: 403, url, headers: new Headers() } as unknown as Response;
      }),
    );
    const data = await fetchOpenGraph("https://www.stockx.com");
    expect(data).toEqual({ imageUrl: "https://www.stockx.com/apple-touch-icon.png" });
  });

  // An SPA typically answers *every* unknown path with its HTML shell, so a
  // 200 alone must never be taken as "this is an icon".
  it("ignores a well-known path that answers with HTML rather than an image", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(async (input: string | URL) => {
        const url = String(input);
        return {
          ok: !url.startsWith("https://spa.example.com/") || url.includes("favicon") || url.includes("apple-touch"),
          status: 403,
          url,
          headers: new Headers({ "content-type": "text/html" }),
        } as unknown as Response;
      }),
    );
    const data = await fetchOpenGraph("https://spa.example.com");
    expect(data).toBeNull();
  });
});

describe("enrichWithOpenGraph", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("skips the fetch entirely when icon/tagline/description are all already set", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const fields = await enrichWithOpenGraph({
      url: "https://example.com",
      iconUrl: "https://example.com/icon.png",
      tagline: "A tagline",
      description: "A description",
    });
    expect(fields).toEqual({
      iconUrl: "https://example.com/icon.png",
      tagline: "A tagline",
      description: "A description",
    });
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("fills only the missing fields, leaving existing ones untouched", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(`
          <meta property="og:image" content="https://example.com/og.png" />
          <meta property="og:title" content="Scraped Title" />
          <meta property="og:description" content="Scraped description" />
        `),
      ),
    );
    const fields = await enrichWithOpenGraph({
      url: "https://example.com",
      iconUrl: null,
      tagline: "Submitter's tagline",
      description: "",
    });
    expect(fields).toEqual({
      iconUrl: "https://example.com/og.png",
      tagline: "Submitter's tagline",
      description: "Scraped description",
    });
  });

  it("falls back to empty/null fields when OpenGraph data is unavailable", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("network down")));
    const fields = await enrichWithOpenGraph({
      url: "https://unreachable.example.com",
      iconUrl: null,
      tagline: "",
      description: "",
    });
    expect(fields).toEqual({ iconUrl: null, tagline: "", description: "" });
  });

  it("truncates scraped title/description to the app's field limits", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        htmlResponse(
          `<meta property="og:title" content="${"T".repeat(200)}" />` +
            `<meta property="og:description" content="${"D".repeat(5000)}" />`,
        ),
      ),
    );
    const fields = await enrichWithOpenGraph({
      url: "https://example.com",
      iconUrl: null,
      tagline: "",
      description: "",
    });
    expect(fields.tagline).toHaveLength(140);
    expect(fields.description).toHaveLength(4000);
  });
});
