// Best-effort fetch of a page's OpenGraph metadata (image/title/description),
// used to auto-fill app presentation (icon, tagline, description) from the
// app's own site when the submitter didn't supply them. Never throws — a
// failed or slow fetch just means falling back to whatever the app already has.

export interface OpenGraphData {
  imageUrl?: string;
  title?: string;
  description?: string;
}

// 10s, not the 5s this started at: measured against the real app list, a
// handful of sites (minecraft.net, christies.com) consistently need more than
// 5s to return their first byte, and the old value turned those into permanent
// icon-less rows.
const FETCH_TIMEOUT_MS = 10_000;
const MAX_HTML_BYTES = 1_000_000; // enough for <head>; avoids reading huge bodies

// Tried in order until one yields metadata. A bot-shaped User-Agent gets
// refused or served a challenge page by a lot of the web:
//
//  1. A plain browser UA. Cloudflare-fronted sites (solflare.com, openai.com)
//     403 an unrecognized crawler outright but serve a normal page to this.
//  2. facebookexternalhit, the canonical link-preview crawler. Some sites
//     (reddit.com, tiktok.com, canva.com, perplexity.ai) go the other way —
//     they gate og: tags behind a *recognized* preview crawler and emit
//     nothing useful for a generic browser UA.
//
// Caveat worth knowing before trusting entry 2: Cloudflare verifies
// known-crawler UAs against the operator's published IP ranges, so claiming to
// be Facebook from a datacenter IP can be treated more harshly than an
// unrecognized UA would be. It's a fallback, tried only after (1) has already
// failed, precisely because it can backfire.
//
// Keep in sync with indexer/src/opengraph.rs's USER_AGENTS.
const USER_AGENTS = [
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
  "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
] as const;

function metaContent(html: string, patterns: RegExp[]): string | undefined {
  for (const pattern of patterns) {
    // Group 1 is the quote character (captured so the content group can stop
    // at a matching close-quote instead of any quote char — content often
    // legitimately contains an apostrophe, e.g. "Solana's ...").
    const match = pattern.exec(html);
    if (match?.[2] !== undefined) return match[2].trim();
  }
  return undefined;
}

// Matches both attribute orders: <meta property="og:x" content="..."> and
// <meta content="..." property="og:x">. The content-value group uses [^>]
// rather than `.` — `.` matches `>`, which let non-greedy backtracking skip
// past this tag's boundary and capture into a *later* meta tag whenever an
// earlier candidate closing-quote didn't satisfy the rest of the pattern
// (e.g. a preceding <meta name="description"> sitting right before the
// og:image tag, as real pages commonly emit with no whitespace between tags).
function metaPatterns(key: string): RegExp[] {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return [
    new RegExp(`<meta[^>]+(?:property|name)=["']${escaped}["'][^>]*content=(["'])([^>]*?)\\1`, "i"),
    new RegExp(`<meta[^>]+content=(["'])([^>]*?)\\1[^>]*(?:property|name)=["']${escaped}["']`, "i"),
  ];
}

function decodeEntities(s: string): string {
  return s
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#0?39;/g, "'");
}

async function readHead(res: Response, maxBytes: number): Promise<string> {
  const reader = res.body?.getReader();
  if (!reader) return await res.text();

  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    total += value.length;
    if (total >= maxBytes) break;
  }
  await reader.cancel().catch(() => {});
  return Buffer.concat(chunks.map((c) => Buffer.from(c))).toString("utf-8");
}

/**
 * Fetch `pageUrl` and extract its OpenGraph (falling back to Twitter card)
 * metadata. Returns null on any network error, non-HTML response, or
 * timeout — callers should treat this as "no data available", not an error.
 *
 * Tries each of USER_AGENTS in turn and takes the first that yields anything,
 * then logs a single line naming what every attempt hit. That log line is the
 * point: this used to swallow all four failure modes silently and identically,
 * so an app that never got an icon was indistinguishable from one whose site
 * has no og: tags at all.
 */
export async function fetchOpenGraph(pageUrl: string): Promise<OpenGraphData | null> {
  const failures: string[] = [];
  for (const userAgent of USER_AGENTS) {
    // The UA is the only thing that differs between attempts, so label each
    // failure with it — "403 as browser, no og: tags as facebookexternalhit"
    // is the shape that tells you which knob (if any) is worth turning next.
    const label = userAgent.startsWith("facebookexternalhit") ? "facebookexternalhit" : "browser";
    const result = await attemptOpenGraph(pageUrl, userAgent);
    if (result.data) return result.data;
    failures.push(`${label}: ${result.failure}`);
  }
  console.warn(`opengraph: no metadata for ${pageUrl} (${failures.join("; ")})`);
  return null;
}

/**
 * One `fetchOpenGraph` attempt with a fixed User-Agent. `failure` is a short
 * human-readable reason, only ever used for the log line above.
 */
async function attemptOpenGraph(
  pageUrl: string,
  userAgent: string,
): Promise<{ data?: OpenGraphData; failure?: string }> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);

  try {
    const res = await fetch(pageUrl, {
      signal: controller.signal,
      headers: {
        "user-agent": userAgent,
        // Sent because a request with no `Accept` at all is itself a bot
        // signal to some WAFs; harmless everywhere else.
        accept: "text/html,application/xhtml+xml,*/*",
      },
      redirect: "follow",
    });
    if (!res.ok) return { failure: `HTTP ${res.status}` };
    const contentType = res.headers.get("content-type") ?? "";
    if (!contentType.includes("html")) return { failure: `non-HTML response (${contentType})` };

    const html = await readHead(res, MAX_HTML_BYTES);

    const rawImage = metaContent(html, [...metaPatterns("og:image"), ...metaPatterns("twitter:image")]);
    const rawTitle = metaContent(html, [...metaPatterns("og:title"), ...metaPatterns("twitter:title")]);
    const rawDescription = metaContent(html, [
      ...metaPatterns("og:description"),
      ...metaPatterns("twitter:description"),
    ]);

    const data: OpenGraphData = {};
    if (rawImage) {
      try {
        data.imageUrl = new URL(decodeEntities(rawImage), res.url).toString();
      } catch {
        // malformed image URL — omit rather than fail the whole fetch
      }
    }
    if (rawTitle) data.title = decodeEntities(rawTitle);
    if (rawDescription) data.description = decodeEntities(rawDescription);

    // A 200 with no usable og:/twitter: tags — the single most common outcome
    // for sites that render their <head> client-side. Reported as a failure so
    // the next User-Agent still gets its turn: some sites emit tags only for a
    // recognized preview crawler.
    if (Object.keys(data).length === 0) return { failure: "no og: or twitter: tags" };
    return { data };
  } catch (e) {
    const aborted = e instanceof Error && e.name === "AbortError";
    return { failure: aborted ? `timeout after ${FETCH_TIMEOUT_MS / 1000}s` : `request failed (${e})` };
  } finally {
    clearTimeout(timeout);
  }
}

// Keep in sync with buildCreateAppTxSchema's tagline/description limits (src/lib/validation.ts).
const TAGLINE_MAX = 140;
const DESCRIPTION_MAX = 4000;

export interface EnrichableApp {
  url: string;
  iconUrl?: string | null;
  tagline?: string | null;
  description?: string | null;
}

export interface EnrichedAppFields {
  iconUrl: string | null;
  tagline: string;
  description: string;
}

/**
 * Fill in whichever of iconUrl/tagline/description the app is missing using
 * its own OpenGraph metadata. Fields the app already has always win over
 * scraped ones; only fetches when at least one field is missing.
 */
export async function enrichWithOpenGraph(app: EnrichableApp): Promise<EnrichedAppFields> {
  const iconUrl = app.iconUrl ?? null;
  const tagline = app.tagline ?? "";
  const description = app.description ?? "";

  if (iconUrl && tagline && description) {
    return { iconUrl, tagline, description };
  }

  const og = await fetchOpenGraph(app.url);
  return {
    iconUrl: iconUrl || og?.imageUrl || null,
    tagline: tagline || (og?.title ?? "").slice(0, TAGLINE_MAX),
    description: description || (og?.description ?? "").slice(0, DESCRIPTION_MAX),
  };
}
