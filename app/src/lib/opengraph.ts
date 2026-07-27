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

// Conventional icon locations, tried (in this order — the Apple one is
// 180x180, the .ico often still 16x16) only after every in-page source has come
// up empty. Worth the extra request because they need no HTML at all: the ~10
// sites that hard-403 this fetcher's page requests mostly still serve their
// static assets to anyone, so this is the only thing that recovers an icon for
// them. Verified against the live app list: 4 of the 403-ing hosts
// (etherscan.io, canva.com, solscan.io, stockx.com) hand these over despite
// refusing their own homepage.
//
// Keep in sync with indexer/src/opengraph.rs's WELL_KNOWN_ICON_PATHS.
const WELL_KNOWN_ICON_PATHS = ["/apple-touch-icon.png", "/favicon.ico"] as const;

// Absolute last resort, once even WELL_KNOWN_ICON_PATHS has failed: a public
// favicon service that already has the icon cached, keyed by host. This is the
// only thing that reaches the handful of sites which 403 both their homepage
// *and* their own static assets (midjourney.com, epicgames.com,
// heritageauctions.com, arkhamintelligence.com).
//
// Two consequences worth being deliberate about, since the resolved URL is
// stored on the App row and then served to every visitor:
//
//  - It is a hotlink. Each card view hits DuckDuckGo, so those viewers' IPs are
//    visible to them. DuckDuckGo rather than Google's s2/favicons specifically
//    to keep that exposure as small as possible — same coverage on all four
//    sites above, no query string, no ad-network operator. Dropping this
//    fallback is a one-line change if that trade isn't wanted; the affected
//    apps simply go back to having no icon.
//  - Icons here are small (~32x32), so this is genuinely worse than every
//    source above it — hence last.
//
// Safe against false positives: the service answers 404 (not a generic globe
// placeholder) for a host it has nothing for, so the res.ok check in
// fallbackIcon is enough to reject it — verified against a nonsense domain and
// against onbtc.multisig.us, both of which correctly 404.
//
// Keep in sync with indexer/src/opengraph.rs's ICON_SERVICE.
const ICON_SERVICE = "https://icons.duckduckgo.com/ip3/";

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

/**
 * The page's <title>, used only when no og:title/twitter:title exists. [^<]
 * rather than a lazy `.` so a page missing its </title> can't run the capture
 * on into the rest of the document.
 */
function titleTag(html: string): string | undefined {
  const raw = /<title[^>]*>([^<]*)<\/title>/i.exec(html)?.[1]?.trim();
  return raw ? raw : undefined;
}

/**
 * Best <link rel="...icon"> href, preferring apple-touch-icon (usually 180x180)
 * over a bare icon/shortcut icon (often still a 16x16 .ico).
 *
 * Parses each <link> tag whole and reads its attributes, rather than
 * pattern-matching rel and href in a fixed order the way metaPatterns has to:
 * rel is a space-separated *list* (rel="shortcut icon"), so matching it as an
 * opaque string would miss half the web. mask-icon is deliberately excluded —
 * it's a monochrome SVG silhouette for Safari's pinned tabs and renders as a
 * black blob anywhere else.
 */
function linkIcon(html: string): string | undefined {
  const attr = (tag: string, name: string) =>
    new RegExp(`\\b${name}\\s*=\\s*["']([^"']*)["']`, "is").exec(tag)?.[1]?.trim();

  let fallback: string | undefined;
  for (const match of html.matchAll(/<link\s[^>]*>/gis)) {
    const tag = match[0];
    const rel = attr(tag, "rel");
    const href = attr(tag, "href");
    if (!rel || !href) continue;
    const rels = rel.split(/\s+/).map((r) => r.toLowerCase());
    if (rels.includes("mask-icon")) continue;
    if (rels.includes("apple-touch-icon") || rels.includes("apple-touch-icon-precomposed")) {
      return href;
    }
    if (!fallback && rels.includes("icon")) fallback = href;
  }
  return fallback;
}

/**
 * Probe WELL_KNOWN_ICON_PATHS against `pageUrl`'s origin and then ICON_SERVICE,
 * returning the first that answers with an actual image.
 *
 * The content-type check is the load-bearing part for the former: a single-page
 * app typically answers *every* unknown path — /favicon.ico included — with its
 * 200 HTML shell, which would otherwise be stored as an icon URL that renders
 * as a broken image.
 */
async function fallbackIcon(pageUrl: string): Promise<string | undefined> {
  let host: string;
  try {
    host = new URL(pageUrl).host;
  } catch {
    return undefined;
  }
  const candidates = [
    ...WELL_KNOWN_ICON_PATHS.map((path) => new URL(path, pageUrl).toString()),
    `${ICON_SERVICE}${host}.ico`,
  ];

  for (const candidate of candidates) {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
    try {
      // GET, not HEAD: plenty of static hosts answer HEAD with 405 while
      // serving the file perfectly well.
      const res = await fetch(candidate, {
        signal: controller.signal,
        headers: { "user-agent": USER_AGENTS[0] },
        redirect: "follow",
      });
      if (res.ok && (res.headers.get("content-type") ?? "").startsWith("image/")) {
        return res.url || candidate;
      }
    } catch {
      // try the next candidate
    } finally {
      clearTimeout(timeout);
    }
  }
  return undefined;
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
  // Held aside across attempts: a <link> icon is better than nothing but worse
  // than an og:image a later User-Agent might still produce.
  let weakIcon: string | undefined;
  let best: OpenGraphData | undefined;

  for (const userAgent of USER_AGENTS) {
    // The UA is the only thing that differs between attempts, so label each
    // failure with it — "403 as browser, no og: tags as facebookexternalhit"
    // is the shape that tells you which knob (if any) is worth turning next.
    const label = userAgent.startsWith("facebookexternalhit") ? "facebookexternalhit" : "browser";
    const result = await attemptOpenGraph(pageUrl, userAgent);
    if (result.failure) {
      failures.push(`${label}: ${result.failure}`);
      continue;
    }
    weakIcon ??= result.linkIcon;
    if (result.data && Object.keys(result.data).length > 0) {
      best = result.data;
      break;
    }
    failures.push(`${label}: no og:, twitter: or title tags`);
  }

  if (!best && !weakIcon) {
    // Every page attempt struck out — the well-known paths need no HTML, so
    // they're the last thing standing for a site that 403s its own homepage.
    const icon = await fallbackIcon(pageUrl);
    if (icon) {
      console.warn(`opengraph: ${pageUrl} served no page (${failures.join("; ")}) — fell back to ${icon}`);
      return { imageUrl: icon };
    }
    console.warn(`opengraph: no metadata for ${pageUrl} (${failures.join("; ")}; no fallback icon either)`);
    return null;
  }

  const data: OpenGraphData = best ?? {};
  // Fill the icon from the weakest sources only if nothing better turned up —
  // a page can perfectly well have og:title but no og:image.
  if (!data.imageUrl) data.imageUrl = weakIcon ?? (await fallbackIcon(pageUrl));
  return Object.keys(data).length > 0 ? data : null;
}

/**
 * One `fetchOpenGraph` attempt with a fixed User-Agent. `failure` is a short
 * human-readable reason, only ever used for the log line above.
 */
async function attemptOpenGraph(
  pageUrl: string,
  userAgent: string,
): Promise<{ data?: OpenGraphData; linkIcon?: string; failure?: string }> {
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
    // <title>/<meta name="description"> as last resorts: a page with no social
    // tags at all still almost always has these two, and a card showing the
    // site's real name beats one showing nothing.
    const rawTitle =
      metaContent(html, [...metaPatterns("og:title"), ...metaPatterns("twitter:title")]) ??
      titleTag(html);
    const rawDescription =
      metaContent(html, [...metaPatterns("og:description"), ...metaPatterns("twitter:description")]) ??
      metaContent(html, metaPatterns("description"));

    const absolute = (raw: string): string | undefined => {
      try {
        return new URL(decodeEntities(raw), res.url).toString();
      } catch {
        return undefined; // malformed URL — omit rather than fail the whole fetch
      }
    };

    const data: OpenGraphData = {};
    if (rawImage) data.imageUrl = absolute(rawImage);
    if (data.imageUrl === undefined) delete data.imageUrl;
    if (rawTitle) data.title = decodeEntities(rawTitle);
    if (rawDescription) data.description = decodeEntities(rawDescription);

    const rawLinkIcon = linkIcon(html);
    return { data, linkIcon: rawLinkIcon ? absolute(rawLinkIcon) : undefined };
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
