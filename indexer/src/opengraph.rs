//! Automatic OpenGraph enrichment for freshly-created apps — fetches the
//! app's own icon/title/description right when `init_app` is observed (see
//! `processors/product.rs::sync_app_from_init`, the only caller of
//! `spawn_enrichment`), instead of requiring the separate
//! `npm run og:backfill` script (`app/scripts/backfillOpengraph.ts`,
//! `app/src/lib/opengraph.ts`) to be run manually. That script still exists
//! as a manual catch-up tool for apps whose live fetch failed outright (site
//! down, timed out, no OG tags at all) — this module is the same extraction
//! logic, just triggered automatically instead of on a schedule.

use regex::Regex;
use sqlx::PgPool;
use std::time::Duration;

// 10s, not the 5s this started at: measured against the real app list, a
// handful of sites (minecraft.net, christies.com) consistently need more
// than 5s to return their first byte, and the old value turned those into
// permanent icon-less rows. Nothing waits on this — enrichment runs in a
// detached task (see spawn_enrichment) — so a slower ceiling costs nothing
// but the task living longer.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Tried in order until one yields metadata. A bot-shaped User-Agent gets
/// refused or served a challenge page by a lot of the web:
///
/// 1. A plain browser UA. Cloudflare-fronted sites (solflare.com,
///    openai.com) 403 an unrecognized crawler outright but serve a normal
///    page to this.
/// 2. `facebookexternalhit`, the canonical link-preview crawler. Some sites
///    (reddit.com, tiktok.com, canva.com, perplexity.ai) go the other way —
///    they gate og: tags behind a *recognized* preview crawler and emit
///    nothing useful for a generic browser UA.
///
/// Caveat worth knowing before trusting entry 2: Cloudflare verifies
/// known-crawler UAs against the operator's published IP ranges, so claiming
/// to be Facebook from a datacenter IP can be treated more harshly than an
/// unrecognized UA would be. It's a fallback, tried only after (1) has
/// already failed, precisely because it can backfire.
///
/// Keep in sync with app/src/lib/opengraph.ts's `USER_AGENTS`.
const USER_AGENTS: [&str; 2] = [
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/126.0.0.0 Safari/537.36",
    "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
];
// Enough for <head>; unlike app/src/lib/opengraph.ts this reads the whole
// response before truncating rather than stopping the stream early — this
// only ever runs once per newly-created app, in a detached background task
// (see spawn_enrichment) that never blocks the crawler, so the bandwidth
// streaming would save isn't worth the extra dependency here.
const MAX_HTML_BYTES: usize = 1_000_000;

// Keep in sync with buildCreateAppTxSchema's tagline/description limits
// (app/src/lib/validation.ts) and app/src/lib/opengraph.ts's own constants —
// this bypasses that Zod schema entirely (server-side only, never goes
// through the create-app HTTP route), so it has to enforce the same bounds
// itself.
const TAGLINE_MAX: usize = 140;
const DESCRIPTION_MAX: usize = 4000;

/// Conventional icon locations, tried (in this order — the Apple one is
/// 180x180, the .ico often still 16x16) only after every in-page source has
/// come up empty. Worth the extra request because they need no HTML at all:
/// the ~10 sites that hard-403 this fetcher's page requests mostly still
/// serve their static assets to anyone, so this is the *only* thing that
/// recovers an icon for them. Verified against the live app list: 4 of the
/// 403-ing hosts (etherscan.io, canva.com, solscan.io, stockx.com) hand
/// these over despite refusing their own homepage.
///
/// Keep in sync with app/src/lib/opengraph.ts's `WELL_KNOWN_ICON_PATHS`.
const WELL_KNOWN_ICON_PATHS: [&str; 2] = ["/apple-touch-icon.png", "/favicon.ico"];

/// Absolute last resort, once even `WELL_KNOWN_ICON_PATHS` has failed: a
/// public favicon service that already has the icon cached, keyed by host.
/// This is the only thing that reaches the handful of sites which 403 both
/// their homepage *and* their own static assets (midjourney.com,
/// epicgames.com, heritageauctions.com, arkhamintelligence.com).
///
/// Two consequences worth being deliberate about, since the resolved URL is
/// stored on the App row and then served to every visitor:
///
/// - It is a hotlink. Each card view hits DuckDuckGo, so those viewers'
///   IPs are visible to them. DuckDuckGo rather than Google's `s2/favicons`
///   specifically to keep that exposure as small as possible — same
///   coverage on all four sites above, no query string, no ad-network
///   operator. Dropping this fallback is a one-line change if that trade
///   isn't wanted; the affected apps simply go back to having no icon.
/// - Icons here are small (~32x32), so this is genuinely worse than every
///   source above it — hence last.
///
/// Safe against false positives: the service answers 404 (not a generic
/// globe placeholder) for a host it has nothing for, so the status check in
/// `fallback_icon` is enough to reject it — verified against a nonsense
/// domain and against onbtc.multisig.us, both of which correctly 404.
///
/// Keep in sync with app/src/lib/opengraph.ts's `ICON_SERVICE`.
const ICON_SERVICE: &str = "https://icons.duckduckgo.com/ip3/";

/// Everything one page fetch yielded. `link_icon` is kept out of `og` so the
/// caller can rank it below a real `og:image` from a later attempt rather
/// than letting whichever User-Agent happened to go first win.
#[derive(Debug, Default)]
struct PageMetadata {
    og: OpenGraphData,
    link_icon: Option<String>,
}

#[derive(Debug, Default)]
struct OpenGraphData {
    image_url: Option<String>,
    title: Option<String>,
    description: Option<String>,
}

impl OpenGraphData {
    fn has_any(&self) -> bool {
        self.image_url.is_some() || self.title.is_some() || self.description.is_some()
    }
}

/// The page's `<title>`, used only when no `og:title`/`twitter:title` exists.
/// `[^<]` rather than a lazy `.` so a page missing its `</title>` can't run
/// the capture on into the rest of the document.
fn title_tag(html: &str) -> Option<String> {
    let pattern = Regex::new(r"(?is)<title[^>]*>([^<]*)</title>").expect("static pattern is valid");
    let raw = pattern.captures(html)?.get(1)?.as_str().trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

/// Best `<link rel="...icon">` href, preferring `apple-touch-icon` (usually
/// 180x180) over a bare `icon`/`shortcut icon` (often still a 16x16 .ico).
///
/// Parses each `<link>` tag whole and reads its attributes, rather than
/// pattern-matching `rel` and `href` in a fixed order the way `meta_patterns`
/// has to: `rel` is a space-separated *list* (`rel="shortcut icon"`), so
/// matching it as an opaque string would miss half the web. `mask-icon` is
/// deliberately excluded — it's a monochrome SVG silhouette for Safari's
/// pinned tabs and renders as a black blob anywhere else.
fn link_icon(html: &str) -> Option<String> {
    let link_tag = Regex::new(r"(?is)<link\s[^>]*>").expect("static pattern is valid");
    let attr = |tag: &str, name: &str| -> Option<String> {
        let pattern = Regex::new(&format!(r#"(?is)\b{name}\s*=\s*["']([^"']*)["']"#))
            .expect("static pattern is valid");
        pattern
            .captures(tag)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
    };

    let mut fallback: Option<String> = None;
    for tag in link_tag.find_iter(html) {
        let tag = tag.as_str();
        let Some(rel) = attr(tag, "rel") else {
            continue;
        };
        let Some(href) = attr(tag, "href").filter(|h| !h.is_empty()) else {
            continue;
        };
        let rels: Vec<&str> = rel.split_whitespace().collect();
        if rels.iter().any(|r| r.eq_ignore_ascii_case("mask-icon")) {
            continue;
        }
        if rels.iter().any(|r| {
            r.eq_ignore_ascii_case("apple-touch-icon")
                || r.eq_ignore_ascii_case("apple-touch-icon-precomposed")
        }) {
            return Some(href);
        }
        if fallback.is_none() && rels.iter().any(|r| r.eq_ignore_ascii_case("icon")) {
            fallback = Some(href);
        }
    }
    fallback
}

/// Matches both attribute orders — `<meta property="og:x" content="...">`
/// and `<meta content="..." property="og:x">` — and both quote styles,
/// mirroring app/src/lib/opengraph.ts's `metaPatterns` (see that file for
/// the full reasoning on the `[^>]` content group), with one deliberate
/// difference: Rust's `regex` crate never supports backreferences (a
/// guaranteed-linear-time engine, unlike JS's backtracking one), so the
/// TS version's single `(["'])...\1` pair — "whichever quote char opened,
/// the same one must close it" — becomes two separate patterns here, one
/// per quote style, instead of one pattern with a backreference.
fn meta_patterns(key: &str) -> [Regex; 4] {
    let escaped = regex::escape(key);
    [
        Regex::new(&format!(
            r#"(?i)<meta[^>]+(?:property|name)=["']{escaped}["'][^>]*content="([^"]*)""#
        ))
        .expect("static pattern is always valid"),
        Regex::new(&format!(
            r#"(?i)<meta[^>]+(?:property|name)=["']{escaped}["'][^>]*content='([^']*)'"#
        ))
        .expect("static pattern is always valid"),
        Regex::new(&format!(
            r#"(?i)<meta[^>]+content="([^"]*)"[^>]*(?:property|name)=["']{escaped}["']"#
        ))
        .expect("static pattern is always valid"),
        Regex::new(&format!(
            r#"(?i)<meta[^>]+content='([^']*)'[^>]*(?:property|name)=["']{escaped}["']"#
        ))
        .expect("static pattern is always valid"),
    ]
}

fn meta_content(html: &str, patterns: &[Regex]) -> Option<String> {
    for pattern in patterns {
        let Some(m) = pattern.captures(html).and_then(|caps| caps.get(1)) else {
            continue;
        };
        let content = m.as_str().trim();
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }
    None
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&#39;", "'")
}

/// Fetch `page_url` and extract its OpenGraph (falling back to Twitter card)
/// metadata. Returns `None` only when every source below came up empty —
/// every caller treats that as "no data available", never as an error to
/// propagate.
///
/// Sources, best first, because a majority of real sites serve *no* og:
/// tags to a non-JS client and the old "og: or nothing" version simply gave
/// up on them:
///
/// 1. `og:`/`twitter:` tags, per User-Agent in `USER_AGENTS`.
/// 2. `<title>` / `<meta name="description">` — a plain page still names
///    itself even with no social tags at all.
/// 3. `<link rel="apple-touch-icon">`, then `<link rel="icon">`.
/// 4. `WELL_KNOWN_ICON_PATHS`, then `ICON_SERVICE` — none of which need
///    any HTML, and so are the only sources that survive a site 403-ing
///    the page fetch outright.
///
/// A weaker source is never allowed to displace a stronger one: a `<link>`
/// icon found under the first UA is held aside and only used if no later
/// attempt turns up a real `og:image`.
///
/// Whatever happens, one line is logged naming what every attempt hit —
/// this used to swallow all its failure modes silently and identically, so
/// an app that never got an icon was indistinguishable from one whose site
/// genuinely has no tags, and telling them apart meant re-probing the site
/// by hand from outside.
async fn fetch_open_graph(http: &reqwest::Client, page_url: &str) -> Option<OpenGraphData> {
    let mut failures = Vec::with_capacity(USER_AGENTS.len());
    // Held aside across attempts: a <link> icon is better than nothing but
    // worse than an og:image a later User-Agent might still produce.
    let mut weak_icon: Option<String> = None;
    let mut best: Option<OpenGraphData> = None;

    for user_agent in USER_AGENTS {
        // The UA is the only thing that differs between attempts, so label
        // each failure with it — "403 as browser, no og: tags as
        // facebookexternalhit" is the shape that tells you which knob (if
        // any) is worth turning next.
        let label = if user_agent.starts_with("facebookexternalhit") {
            "facebookexternalhit"
        } else {
            "browser"
        };
        match try_fetch_open_graph(http, page_url, user_agent).await {
            Ok(page) => {
                if weak_icon.is_none() {
                    weak_icon = page.link_icon;
                }
                if page.og.has_any() {
                    best = Some(page.og);
                    break;
                }
                failures.push(format!("{label}: no og:, twitter: or title tags"));
            }
            Err(reason) => failures.push(format!("{label}: {reason}")),
        }
    }

    let mut data = match best {
        Some(data) => data,
        // Every page attempt struck out. A held-aside <link> icon still
        // counts as a result — an icon and nothing else is exactly what
        // `apply_metadata_update` is built to merge in.
        None if weak_icon.is_some() => OpenGraphData::default(),
        None => {
            let icon = fallback_icon(http, page_url).await;
            return match icon {
                Some(url) => {
                    log::info!(
                        "opengraph: {page_url} served no page ({}) — fell back to {url}",
                        failures.join("; ")
                    );
                    Some(OpenGraphData {
                        image_url: Some(url),
                        ..Default::default()
                    })
                }
                None => {
                    log::info!(
                        "opengraph: no metadata for {page_url} ({}; no fallback icon either)",
                        failures.join("; ")
                    );
                    None
                }
            };
        }
    };

    // Fill the icon from the weakest sources only if nothing better turned
    // up — a page can perfectly well have og:title but no og:image.
    if data.image_url.is_none() {
        data.image_url = match weak_icon {
            Some(icon) => Some(icon),
            None => fallback_icon(http, page_url).await,
        };
    }
    Some(data)
}

/// Probe `WELL_KNOWN_ICON_PATHS` against `page_url`'s origin and then
/// `ICON_SERVICE`, returning the first that answers with an actual image.
///
/// The content-type check is the load-bearing part for the former: a
/// single-page app typically answers *every* unknown path — `/favicon.ico`
/// included — with its 200 HTML shell, which would otherwise be stored as an
/// icon URL that renders as a broken image.
async fn fallback_icon(http: &reqwest::Client, page_url: &str) -> Option<String> {
    let base = reqwest::Url::parse(page_url).ok()?;
    let mut candidates: Vec<reqwest::Url> = WELL_KNOWN_ICON_PATHS
        .iter()
        .filter_map(|path| base.join(path).ok())
        .collect();
    if let Some(host) = base.host_str() {
        if let Ok(url) = reqwest::Url::parse(&format!("{ICON_SERVICE}{host}.ico")) {
            candidates.push(url);
        }
    }

    for candidate in candidates {
        // GET, not HEAD: plenty of static hosts answer HEAD with 405 while
        // serving the file perfectly well. The body is never read — these
        // are small, and dropping the response closes the connection.
        let Ok(res) = http
            .get(candidate.clone())
            .header(reqwest::header::USER_AGENT, USER_AGENTS[0])
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
        else {
            continue;
        };
        if !res.status().is_success() {
            continue;
        }
        let is_image = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.starts_with("image/"));
        if is_image {
            return Some(res.url().to_string());
        }
    }
    None
}

/// One `fetch_open_graph` attempt with a fixed User-Agent. The `Err` string
/// is a short human-readable reason, only ever used for the log line above.
async fn try_fetch_open_graph(
    http: &reqwest::Client,
    page_url: &str,
    user_agent: &str,
) -> Result<PageMetadata, String> {
    let res = http
        .get(page_url)
        .header(reqwest::header::USER_AGENT, user_agent)
        // Sent because a request with no `Accept` at all is itself a bot
        // signal to some WAFs; harmless everywhere else.
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,*/*",
        )
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!("timeout after {}s", FETCH_TIMEOUT.as_secs())
            } else {
                format!("request failed ({e})")
            }
        })?;

    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !content_type.contains("html") {
        return Err(format!("non-HTML response ({content_type})"));
    }
    let final_url = res.url().clone();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| format!("body read failed ({e})"))?;
    let cutoff = bytes.len().min(MAX_HTML_BYTES);
    let html = String::from_utf8_lossy(&bytes[..cutoff]);

    let image_pats = [meta_patterns("og:image"), meta_patterns("twitter:image")].concat();
    let title_pats = [meta_patterns("og:title"), meta_patterns("twitter:title")].concat();
    let description_pats = [
        meta_patterns("og:description"),
        meta_patterns("twitter:description"),
    ]
    .concat();

    let absolute = |raw: String| -> Option<String> {
        final_url
            .join(&decode_entities(&raw))
            .ok()
            .map(|u| u.to_string())
    };

    let data = OpenGraphData {
        image_url: meta_content(&html, &image_pats).and_then(absolute),
        // `<title>`/`<meta name="description">` as last resorts: a page with
        // no social tags at all still almost always has these two, and a card
        // showing the site's real name beats one showing nothing.
        title: meta_content(&html, &title_pats)
            .or_else(|| title_tag(&html))
            .map(|raw| decode_entities(&raw)),
        description: meta_content(&html, &description_pats)
            .or_else(|| meta_content(&html, &meta_patterns("description")))
            .map(|raw| decode_entities(&raw)),
    };

    Ok(PageMetadata {
        link_icon: link_icon(&html).and_then(absolute),
        og: data,
    })
}

/// Fetches `url`'s OpenGraph metadata and fills in whichever of icon/
/// tagline/description `app_id`'s row is still missing via
/// `product::apply_metadata_update` — existing values (from the on-chain
/// memo, or a previous enrichment) always win, same as
/// `backfillOpengraph.ts`'s `enrichWithOpenGraph`. Never propagates an
/// error: every failure mode here (network, timeout, no OG tags found, DB
/// write failure) is swallowed and logged, since this only ever runs
/// detached from the crawler tick that spawned it (see `spawn_enrichment`)
/// — nothing is waiting on the result.
async fn enrich_app(pool: &PgPool, http: &reqwest::Client, app_id: &str, url: &str) {
    let Some(og) = fetch_open_graph(http, url).await else {
        return;
    };

    let tagline = og
        .title
        .map(|t| t.trim().chars().take(TAGLINE_MAX).collect::<String>());
    let description = og
        .description
        .map(|d| d.trim().chars().take(DESCRIPTION_MAX).collect::<String>());

    if og.image_url.is_none() && tagline.is_none() && description.is_none() {
        return;
    }

    match crate::processors::product::apply_metadata_update(
        pool,
        app_id,
        og.image_url.as_deref(),
        tagline.as_deref(),
        description.as_deref(),
    )
    .await
    {
        Ok(()) => log::info!("opengraph: enriched app {app_id} from {url}"),
        Err(e) => log::warn!("opengraph: failed to save enrichment for app {app_id}: {e}"),
    }
}

/// Spawns `enrich_app` as a detached background task — never awaited by the
/// crawler, so a slow or failed OpenGraph fetch can't stall indexing. A
/// no-op if the on-chain memo already supplied everything a card needs.
/// Called right after a new App row lands (see
/// `processors/product.rs::sync_app_from_init`).
pub fn spawn_enrichment(
    pool: PgPool,
    http: reqwest::Client,
    app_id: String,
    url: String,
    needs_icon: bool,
    needs_tagline: bool,
    needs_description: bool,
) {
    if !needs_icon && !needs_tagline && !needs_description {
        return;
    }
    tokio::spawn(async move {
        enrich_app(&pool, &http, &app_id, &url).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for a live-only failure mode that unit tests should
    /// have caught in the first place: Rust's `regex` crate rejects
    /// backreferences at *compile* time (`Regex::new` returns `Err`, not a
    /// panic at match time) — the very first attempt to compile a JS-style
    /// `(["'])...\1` pattern here panicked every enrichment task in
    /// production instead of failing this test. Exercising `meta_patterns`
    /// for every key this module actually calls it with is enough to catch
    /// that class of bug before it ships.
    #[test]
    fn meta_patterns_compiles_for_every_key_this_module_uses() {
        for key in [
            "og:image",
            "twitter:image",
            "og:title",
            "twitter:title",
            "og:description",
            "twitter:description",
        ] {
            meta_patterns(key);
        }
    }

    fn all_patterns(key: &str) -> Vec<Regex> {
        meta_patterns(key).to_vec()
    }

    /// Same class of bug as the test above, for the patterns `link_icon` and
    /// `title_tag` build: both compile regexes at call time, so a malformed
    /// one panics the enrichment task rather than failing to build.
    #[test]
    fn link_and_title_patterns_compile() {
        link_icon("");
        title_tag("");
    }

    #[test]
    fn link_icon_prefers_apple_touch_icon_over_a_plain_icon() {
        let html = r#"<link rel="icon" href="/small.ico">
                      <link rel="apple-touch-icon" href="/big.png">"#;
        assert_eq!(link_icon(html), Some("/big.png".to_string()));
    }

    /// `rel` is a space-separated list, so the common `rel="shortcut icon"`
    /// spelling has to match the same as a bare `rel="icon"`.
    #[test]
    fn link_icon_matches_shortcut_icon_in_a_rel_list() {
        let html = r#"<link rel="shortcut icon" href="/fav.ico">"#;
        assert_eq!(link_icon(html), Some("/fav.ico".to_string()));
    }

    /// Safari's pinned-tab icon is a monochrome silhouette — usable as a
    /// site icon nowhere else, so it must never be picked up.
    #[test]
    fn link_icon_ignores_mask_icon() {
        let html = r##"<link rel="mask-icon" href="/mask.svg" color="#000">"##;
        assert_eq!(link_icon(html), None);
    }

    #[test]
    fn link_icon_handles_href_before_rel_and_single_quotes() {
        let html = r#"<link href='/a.png' rel='apple-touch-icon'>"#;
        assert_eq!(link_icon(html), Some("/a.png".to_string()));
    }

    #[test]
    fn link_icon_returns_none_when_the_only_link_is_a_stylesheet() {
        let html = r#"<link rel="stylesheet" href="/app.css">"#;
        assert_eq!(link_icon(html), None);
    }

    #[test]
    fn title_tag_reads_the_document_title() {
        assert_eq!(
            title_tag("<head><title>  Example Site  </title></head>"),
            Some("Example Site".to_string())
        );
    }

    /// An unclosed `<title>` must not swallow the rest of the document.
    #[test]
    fn title_tag_returns_none_when_unterminated() {
        assert_eq!(title_tag("<title>no closing tag here"), None);
    }

    #[test]
    fn title_tag_returns_none_for_an_empty_title() {
        assert_eq!(title_tag("<title>   </title>"), None);
    }

    #[test]
    fn meta_content_extracts_double_quoted_property_then_content() {
        let html = r#"<meta property="og:image" content="https://example.com/a.png">"#;
        assert_eq!(
            meta_content(html, &all_patterns("og:image")),
            Some("https://example.com/a.png".to_string())
        );
    }

    #[test]
    fn meta_content_extracts_single_quoted_property_then_content() {
        let html = r#"<meta property='og:image' content='https://example.com/a.png'>"#;
        assert_eq!(
            meta_content(html, &all_patterns("og:image")),
            Some("https://example.com/a.png".to_string())
        );
    }

    #[test]
    fn meta_content_extracts_content_then_property_order() {
        let html = r#"<meta content="https://example.com/a.png" property="og:image">"#;
        assert_eq!(
            meta_content(html, &all_patterns("og:image")),
            Some("https://example.com/a.png".to_string())
        );
    }

    #[test]
    fn meta_content_extracts_name_attribute_not_just_property() {
        let html = r#"<meta name="twitter:title" content="Some Title">"#;
        assert_eq!(
            meta_content(html, &all_patterns("twitter:title")),
            Some("Some Title".to_string())
        );
    }

    /// The exact case a JS-style backreference pattern exists to handle:
    /// double-quoted content containing an unescaped apostrophe must not
    /// terminate early at that apostrophe.
    #[test]
    fn meta_content_handles_an_apostrophe_inside_double_quoted_content() {
        let html = r#"<meta property="og:description" content="Solana's fastest DEX">"#;
        assert_eq!(
            meta_content(html, &all_patterns("og:description")),
            Some("Solana's fastest DEX".to_string())
        );
    }

    #[test]
    fn meta_content_returns_none_when_key_is_absent() {
        let html = r#"<meta property="og:title" content="Some Title">"#;
        assert_eq!(meta_content(html, &all_patterns("og:image")), None);
    }

    #[test]
    fn meta_content_returns_none_for_empty_content() {
        let html = r#"<meta property="og:image" content="">"#;
        assert_eq!(meta_content(html, &all_patterns("og:image")), None);
    }

    #[test]
    fn decode_entities_covers_every_replacement() {
        assert_eq!(
            decode_entities("A &amp; B &lt;tag&gt; &quot;q&quot; it&#39;s it&#039;s"),
            "A & B <tag> \"q\" it's it's"
        );
    }
}
