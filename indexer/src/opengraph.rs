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

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
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

#[derive(Debug, Default)]
struct OpenGraphData {
    image_url: Option<String>,
    title: Option<String>,
    description: Option<String>,
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
/// metadata. Returns `None` on any network error, non-HTML response, or
/// timeout, or if neither found anything — every caller treats this as "no
/// data available", never as an error to propagate.
async fn fetch_open_graph(http: &reqwest::Client, page_url: &str) -> Option<OpenGraphData> {
    let res = http
        .get(page_url)
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (compatible; AppMapBot/1.0; +https://appmap)",
        )
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        return None;
    }
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !content_type.contains("html") {
        return None;
    }
    let final_url = res.url().clone();
    let bytes = res.bytes().await.ok()?;
    let cutoff = bytes.len().min(MAX_HTML_BYTES);
    let html = String::from_utf8_lossy(&bytes[..cutoff]);

    let image_pats = [meta_patterns("og:image"), meta_patterns("twitter:image")].concat();
    let title_pats = [meta_patterns("og:title"), meta_patterns("twitter:title")].concat();
    let description_pats = [
        meta_patterns("og:description"),
        meta_patterns("twitter:description"),
    ]
    .concat();

    let data = OpenGraphData {
        image_url: meta_content(&html, &image_pats).and_then(|raw| {
            final_url
                .join(&decode_entities(&raw))
                .ok()
                .map(|u| u.to_string())
        }),
        title: meta_content(&html, &title_pats).map(|raw| decode_entities(&raw)),
        description: meta_content(&html, &description_pats).map(|raw| decode_entities(&raw)),
    };

    if data.image_url.is_none() && data.title.is_none() && data.description.is_none() {
        None
    } else {
        Some(data)
    }
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
