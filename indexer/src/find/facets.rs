//! Catalog rows to `Candidate` facet vectors — the IO-adjacent read adapter.

use crate::find::params;
use crate::find::{Candidate, FacetKind, FacetRef};
use crate::handlers::apps::AppDto;
use std::collections::HashMap;

/// The 16 fixed category slugs. Duplicated from `app/src/lib/constants.ts`'s
/// `CATEGORIES` on purpose (A34): the engine is Rust, the catalog's canonical
/// list is a TypeScript constant, and `app/` cannot reach the indexer's
/// vocabulary at build time. This file is the declared owner of that
/// duplication — change both together.
pub const CATEGORY_SLUGS: [&str; 16] = [
    "defi",
    "nft",
    "gaming",
    "dao",
    "infrastructure",
    "wallet",
    "social",
    "payments",
    "analytics",
    "developer-tools",
    "marketplace",
    "productivity",
    "design",
    "ai",
    "entertainment",
    "other",
];

/// The 8 fixed chain slugs, mirroring `app/src/lib/constants.ts`'s `CHAINS`.
/// Same duplication contract as `CATEGORY_SLUGS`.
pub const CHAIN_SLUGS: [&str; 8] = [
    "solana", "ethereum", "base", "polygon", "bitcoin", "aptos", "sui", "web2",
];

/// A plain-language question per category slug, aimed at a visitor with no
/// crypto fluency (brief success criterion 1). "Is the category defi?" is
/// useless to the person this funnel exists for; these are what the question
/// means to someone who has never held a token. Keyed by slug rather than
/// derived from it because the phrasing is editorial, not mechanical.
///
/// Every one asks what the visitor WANTS, never what the app IS (A78). The
/// visitor arrives with a need and no app in mind, so "Is it a game?" asks them
/// to describe the thing they came here to find. Keep new entries short enough
/// to read as a single tappable question, and keep the "Do you ..." opening —
/// the whole funnel, chains and tags included, speaks in one voice.
const CATEGORY_PROMPTS: [(&str, &str); 16] = [
    (
        "defi",
        "Do you want to trade, lend, or earn yield on your money?",
    ),
    ("nft", "Do you want to collect or trade digital art?"),
    ("gaming", "Do you want something to play?"),
    (
        "dao",
        "Do you want to join a group that votes on decisions together?",
    ),
    (
        "infrastructure",
        "Do you want building blocks for other apps, rather than something to use yourself?",
    ),
    ("wallet", "Do you need somewhere to hold and send money?"),
    ("social", "Do you want to talk to and follow other people?"),
    ("payments", "Do you need to pay someone, or to get paid?"),
    ("analytics", "Do you want to track numbers and see charts?"),
    (
        "developer-tools",
        "Do you need tools for building software?",
    ),
    ("marketplace", "Do you want to buy or sell things?"),
    ("productivity", "Do you want help getting your work done?"),
    ("design", "Do you want to make visuals, art, or layouts?"),
    ("ai", "Do you want AI to do the work for you?"),
    (
        "entertainment",
        "Do you want something to watch or listen to?",
    ),
    (
        "other",
        "Do you want something unusual that fits none of these?",
    ),
];

/// Human-readable label per chain slug, read inside "Do you need it to work on
/// {label}?" so the chain questions carry the same need-framing as the
/// categories (A78). `web2` is the one that cannot be title-cased mechanically
/// — to the target visitor it is not a chain name at all, it is "no crypto
/// required", and phrasing it as "Web2" would ask a question only an insider
/// could answer.
const CHAIN_LABELS: [(&str, &str); 8] = [
    ("solana", "Solana"),
    ("ethereum", "Ethereum"),
    ("base", "Base"),
    ("polygon", "Polygon"),
    ("bitcoin", "Bitcoin"),
    ("aptos", "Aptos"),
    ("sui", "Sui"),
    ("web2", "the regular web, with no crypto wallet"),
];

/// What every candidate scores when `rank_score` has no spread at all — the
/// midpoint of `[0, 1]`, not `0.0`. Today there are zero logged sessions, so
/// the learned term is inert and the content term carries the whole product;
/// collapsing a degenerate spread to zero would silently delete the only
/// signal the funnel has. Not a tunable — it is the midpoint by definition.
const FLAT_CONTENT_SCORE: f64 = 0.5;

/// Build scoring candidates from catalog rows. `learned` maps app id to
/// (support `v_i`, outcome mean `R_i`); an app absent from it gets `(0.0, 0.0)`,
/// which the shrinkage in `blend` reads as "no history".
pub fn candidates_from_apps(
    apps: &[AppDto],
    learned: &HashMap<String, (f64, f64)>,
) -> Vec<Candidate> {
    let (min, max) = apps
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), app| {
            (min.min(app.rank_score), max.max(app.rank_score))
        });
    let spread = max - min;

    apps.iter()
        .map(|app| {
            let (support, outcome_mean) = learned.get(&app.id).copied().unwrap_or((0.0, 0.0));
            Candidate {
                app_id: app.id.clone(),
                category: app.category.clone(),
                chain: app.chain.clone(),
                // Tag slugs are already canonical — re-slugifying here would
                // silently desync the pool from what `AppTag` actually stores.
                tags: app.tags.iter().map(|tag| tag.slug.clone()).collect(),
                content_score: if spread > 0.0 {
                    (app.rank_score - min) / spread
                } else {
                    FLAT_CONTENT_SCORE
                },
                support,
                outcome_mean,
            }
        })
        .collect()
}

/// Every facet worth asking about. *Which* one to ask is `selection`'s job —
/// this is only the vocabulary.
///
/// Categories and chains are included unconditionally, even when no candidate
/// carries the value: "do you want something to play?" asked of a set with no
/// games is highly informative, and its answer is a true `Absent` for everyone.
pub fn facet_pool(candidates: &[Candidate]) -> Vec<FacetRef> {
    let mut pool: Vec<FacetRef> = CATEGORY_SLUGS
        .iter()
        .map(|slug| FacetRef {
            kind: FacetKind::Category,
            value: (*slug).to_string(),
        })
        .chain(CHAIN_SLUGS.iter().map(|slug| FacetRef {
            kind: FacetKind::Chain,
            value: (*slug).to_string(),
        }))
        .collect();

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for candidate in candidates {
        for tag in &candidate.tags {
            *counts.entry(tag.as_str()).or_insert(0) += 1;
        }
    }

    // A tag on one or two apps cannot usefully split the candidate set, but
    // still costs a full expected-information-gain evaluation every turn (A34).
    let mut tags: Vec<&str> = counts
        .into_iter()
        .filter(|(_, count)| *count >= params::MIN_TAG_SUPPORT)
        .map(|(tag, _)| tag)
        .collect();
    // `HashSet`/`HashMap` iteration order is not stable across runs, and an
    // unstable pool would make question choice non-reproducible for identical
    // input — which would make any bug report about the funnel unfalsifiable.
    tags.sort_unstable();

    pool.extend(tags.into_iter().map(|tag| FacetRef {
        kind: FacetKind::Tag,
        value: tag.to_string(),
    }));
    pool
}

/// Human phrasing for a facet (A10). Total: an unrecognised slug still gets a
/// usable question, because tag slugs are crowd-sourced and unbounded.
pub fn prompt_for(facet: &FacetRef) -> String {
    match facet.kind {
        FacetKind::Category => CATEGORY_PROMPTS
            .iter()
            .find(|(slug, _)| *slug == facet.value)
            .map(|(_, prompt)| (*prompt).to_string())
            .unwrap_or_else(|| format!("Do you want something in {}?", humanize(&facet.value))),
        FacetKind::Chain => {
            let label = CHAIN_LABELS
                .iter()
                .find(|(slug, _)| *slug == facet.value)
                .map(|(_, label)| (*label).to_string())
                .unwrap_or_else(|| humanize(&facet.value));
            format!("Do you need it to work on {label}?")
        }
        FacetKind::Tag => format!(
            "Do you want something to do with \"{}\"?",
            humanize(&facet.value)
        ),
    }
}

fn humanize(slug: &str) -> String {
    slug.replace('-', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::FacetState;
    use crate::handlers::apps::TagDto;
    use std::collections::HashSet;

    fn tag(slug: &str) -> TagDto {
        TagDto {
            id: format!("apptag-{slug}"),
            tag_id: format!("tag-{slug}"),
            slug: slug.to_string(),
            name: slug.to_string(),
            stake_total: 0.0,
            suggested_by: None,
        }
    }

    fn app(id: &str, category: &str, chain: &str, tags: &[&str], rank_score: f64) -> AppDto {
        AppDto {
            id: id.to_string(),
            slug: id.to_string(),
            name: id.to_string(),
            tagline: String::new(),
            description: String::new(),
            url: String::new(),
            icon_url: None,
            category: category.to_string(),
            chain: chain.to_string(),
            status: "approved".to_string(),
            created_at: String::new(),
            submitted_by: None,
            vote_count: 0,
            vote_weight: 0.0,
            stake_total: 0.0,
            view_count: 0,
            rank_score,
            tags: tags.iter().map(|slug| tag(slug)).collect(),
            trend: None,
        }
    }

    fn candidate(id: &str, category: &str, tags: &[&str]) -> Candidate {
        Candidate {
            app_id: id.to_string(),
            category: category.to_string(),
            chain: "solana".to_string(),
            tags: tags.iter().map(|slug| slug.to_string()).collect(),
            content_score: 0.5,
            support: 0.0,
            outcome_mean: 0.0,
        }
    }

    #[test]
    fn maps_facets_through_unchanged_and_preserves_order() {
        let apps = [
            app("a", "defi", "solana", &["lending", "dex"], 1.0),
            app("b", "gaming", "base", &[], 2.0),
        ];
        let candidates = candidates_from_apps(&apps, &HashMap::new());

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].app_id, "a");
        assert_eq!(candidates[0].category, "defi");
        assert_eq!(candidates[0].chain, "solana");
        assert_eq!(
            candidates[0].tags,
            HashSet::from(["lending".to_string(), "dex".to_string()])
        );
        assert_eq!(candidates[1].app_id, "b");
        assert_eq!(candidates[1].category, "gaming");
        assert_eq!(candidates[1].chain, "base");
        assert!(candidates[1].tags.is_empty());
    }

    #[test]
    fn content_score_is_min_max_normalized() {
        let apps = [
            app("a", "defi", "solana", &[], 0.0),
            app("b", "defi", "solana", &[], 5.0),
            app("c", "defi", "solana", &[], 10.0),
        ];
        let candidates = candidates_from_apps(&apps, &HashMap::new());
        let scores: Vec<f64> = candidates.iter().map(|c| c.content_score).collect();

        for (got, want) in scores.iter().zip([0.0, 0.5, 1.0]) {
            assert!((got - want).abs() < 1e-12, "got {scores:?}");
        }
    }

    /// The cold-start case: with zero logged sessions the content term carries
    /// the entire product, so a degenerate `rank_score` spread must not divide
    /// by zero or collapse every candidate to 0.0.
    #[test]
    fn identical_rank_scores_all_normalize_to_one_half() {
        for rank_score in [0.0, 7.5] {
            let apps = [
                app("a", "defi", "solana", &[], rank_score),
                app("b", "nft", "base", &[], rank_score),
            ];
            let candidates = candidates_from_apps(&apps, &HashMap::new());
            for candidate in &candidates {
                assert!(candidate.content_score.is_finite());
                assert!((candidate.content_score - 0.5).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn learned_values_land_on_the_right_candidate() {
        let apps = [
            app("a", "defi", "solana", &[], 1.0),
            app("b", "nft", "base", &[], 2.0),
        ];
        let learned = HashMap::from([("b".to_string(), (12.0, 0.75))]);
        let candidates = candidates_from_apps(&apps, &learned);

        assert_eq!(candidates[0].support, 0.0);
        assert_eq!(candidates[0].outcome_mean, 0.0);
        assert_eq!(candidates[1].support, 12.0);
        assert_eq!(candidates[1].outcome_mean, 0.75);
    }

    /// Categories and chains are asked about unconditionally: "do you want
    /// something to play?" over a set containing no games is a legitimate,
    /// highly informative question whose answer is `Absent` for every candidate.
    #[test]
    fn pool_carries_every_category_and_chain_even_when_empty() {
        let pool = facet_pool(&[]);

        for slug in CATEGORY_SLUGS {
            assert!(pool.contains(&FacetRef {
                kind: FacetKind::Category,
                value: slug.to_string(),
            }));
        }
        for slug in CHAIN_SLUGS {
            assert!(pool.contains(&FacetRef {
                kind: FacetKind::Chain,
                value: slug.to_string(),
            }));
        }
        assert_eq!(pool.len(), CATEGORY_SLUGS.len() + CHAIN_SLUGS.len());
    }

    #[test]
    fn tags_below_min_support_are_excluded() {
        let below: Vec<Candidate> = (0..params::MIN_TAG_SUPPORT - 1)
            .map(|i| candidate(&format!("a{i}"), "defi", &["thin"]))
            .collect();
        let at: Vec<Candidate> = (0..params::MIN_TAG_SUPPORT)
            .map(|i| candidate(&format!("b{i}"), "defi", &["popular"]))
            .collect();

        let thin = FacetRef {
            kind: FacetKind::Tag,
            value: "thin".to_string(),
        };
        let popular = FacetRef {
            kind: FacetKind::Tag,
            value: "popular".to_string(),
        };

        assert!(!facet_pool(&below).contains(&thin));
        assert!(facet_pool(&at).contains(&popular));
    }

    #[test]
    fn pool_is_deterministic_and_free_of_duplicates() {
        let candidates = [
            candidate("a", "defi", &["lending", "dex"]),
            candidate("b", "defi", &["lending", "dex"]),
            candidate("c", "defi", &["lending", "dex", "defi"]),
        ];

        let first = facet_pool(&candidates);
        let second = facet_pool(&candidates);
        assert_eq!(first, second);

        let unique: HashSet<&FacetRef> = first.iter().collect();
        assert_eq!(unique.len(), first.len());
    }

    #[test]
    fn every_category_has_its_own_plain_language_prompt() {
        let prompts: Vec<String> = CATEGORY_SLUGS
            .iter()
            .map(|slug| {
                prompt_for(&FacetRef {
                    kind: FacetKind::Category,
                    value: slug.to_string(),
                })
            })
            .collect();

        assert!(prompts.iter().all(|p| !p.trim().is_empty()));
        // Distinctness is the real check: a copy-pasted or missing entry
        // silently collapses two very different questions into one.
        let distinct: HashSet<&String> = prompts.iter().collect();
        assert_eq!(distinct.len(), CATEGORY_SLUGS.len());

        for slug in CHAIN_SLUGS {
            assert!(!prompt_for(&FacetRef {
                kind: FacetKind::Chain,
                value: slug.to_string(),
            })
            .trim()
            .is_empty());
        }

        // Unknown values must degrade to a sensible string, never panic: tag
        // slugs are crowd-sourced and unbounded.
        let unknown_tag = prompt_for(&FacetRef {
            kind: FacetKind::Tag,
            value: "yield-farming".to_string(),
        });
        assert!(unknown_tag.contains("yield farming"));
        assert!(!prompt_for(&FacetRef {
            kind: FacetKind::Category,
            value: "not-a-real-category".to_string(),
        })
        .trim()
        .is_empty());
        assert!(!prompt_for(&FacetRef {
            kind: FacetKind::Chain,
            value: "not-a-real-chain".to_string(),
        })
        .trim()
        .is_empty());
    }

    /// A78: every prompt asks what the visitor WANTS, never what the app IS.
    /// The funnel's premise is that the visitor has a need and no app in mind,
    /// so a question they can only answer by describing the thing they came to
    /// find is unanswerable by construction.
    #[test]
    fn a78_every_prompt_asks_about_the_need_not_the_app() {
        let mut prompts: Vec<String> = CATEGORY_SLUGS
            .iter()
            .map(|slug| {
                prompt_for(&FacetRef {
                    kind: FacetKind::Category,
                    value: slug.to_string(),
                })
            })
            .collect();
        prompts.extend(CHAIN_SLUGS.iter().map(|slug| {
            prompt_for(&FacetRef {
                kind: FacetKind::Chain,
                value: slug.to_string(),
            })
        }));
        // The three unbounded/fallback paths speak the same voice as the
        // authored maps, or the funnel changes register mid-session.
        for kind in [FacetKind::Category, FacetKind::Chain, FacetKind::Tag] {
            prompts.push(prompt_for(&FacetRef {
                kind,
                value: "yield-farming".to_string(),
            }));
        }

        for prompt in &prompts {
            assert!(prompt.starts_with("Do you "), "not need-framed: {prompt:?}");
        }
    }

    /// A14/A21: the three-valued rule survives the adapter. A candidate built
    /// from a catalog row with no tags is `Absent` for the categories it is
    /// not, but `Unknown` — never `Absent` — for a tag nobody applied to it.
    #[test]
    fn a14_a21_untagged_candidate_is_unknown_not_absent_for_tags() {
        let candidates =
            candidates_from_apps(&[app("a", "defi", "solana", &[], 1.0)], &HashMap::new());
        let candidate = &candidates[0];

        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Category,
                value: "defi".to_string(),
            }),
            FacetState::Present
        );
        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Category,
                value: "gaming".to_string(),
            }),
            FacetState::Absent
        );
        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Tag,
                value: "lending".to_string(),
            }),
            FacetState::Unknown
        );
    }
}
