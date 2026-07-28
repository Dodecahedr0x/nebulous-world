//! The `/find` funnel engine: a guided app-discovery flow that asks a short
//! series of maximally-discriminating questions and converges on one suggestion.
//!
//! `scoring`, `selection` and `blend` are **pure** — no `sqlx`, no `axum`, no
//! `ApiState`. They see only the plain data declared below, which is what lets
//! the maths be unit-tested with no database and no validator, mirroring
//! `programs/nebulous_world/src/reward_math.rs`. `facets` and `store` are the
//! two IO-adjacent adapters: `facets` turns catalog rows into `Candidate`s,
//! `store` reads and writes session outcomes. Keeping the split intact is a
//! hard requirement, not a preference — see the repo root `AGENTS.md`.
//!
//! Every tunable lives in `params`; there are no magic numbers elsewhere here.

pub mod blend;
pub mod facets;
pub mod params;
pub mod scoring;
pub mod selection;
pub mod store;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FacetRef {
    pub kind: FacetKind,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum FacetKind {
    Category,
    Chain,
    Tag,
}

/// `Skip` ("don't care") performs no update at all rather than a damped one —
/// the one point both documented precedents agree on. Hedged answers are
/// deliberately absent: no citable likelihood exists for them (A15).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AnswerValue {
    Yes,
    No,
    Skip,
}

/// Three-valued (A14). `Unknown` is NOT `Absent` — see `Candidate::state`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FacetState {
    Present,
    Absent,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub facet: FacetRef,
    pub value: AnswerValue,
}

/// One scoring candidate. Plain data: no `sqlx`, no `axum`, no row types —
/// `scoring`, `selection` and `blend` see only this, which is what makes them
/// unit-testable with no database. `facets::candidates_from_apps` is the only
/// thing that builds one from a catalog row.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub app_id: String,
    /// Exactly one, always present — `App.category` is NOT NULL DEFAULT 'other'.
    pub category: String,
    /// Exactly one, always present — `App.chain` is NOT NULL DEFAULT 'solana'.
    pub chain: String,
    /// Crowd-sourced and sparse. Absence here means "nobody tagged it", not "no".
    pub tags: HashSet<String>,
    /// App-data quality prior, normalized to `[0, 1]` by `facets`.
    pub content_score: f64,
    /// Learned support count `v_i` — completed `/find` outcomes naming this app.
    pub support: f64,
    /// Learned mean outcome `R_i` in `[0, 1]`; meaningless when `support == 0`.
    pub outcome_mean: f64,
}

impl Candidate {
    /// The single load-bearing three-valued rule (A21). Whether a non-match is
    /// `Absent` or `Unknown` is a property of the schema, not a modelling
    /// preference:
    ///
    /// - `Category` and `Chain` are **total** — `App.category` and `App.chain`
    ///   are NOT NULL with defaults, so a row carries exactly one of each and
    ///   every other value is a genuine `Absent`.
    /// - `Tag` is **sparse** — `AppTag` rows are opt-in, so a tag this app does
    ///   not carry means "nobody tagged it", i.e. `Unknown`, never `Absent`.
    ///
    /// That last case is the one mechanism that stops well-tagged apps from
    /// dominating thinly-tagged ones. Collapsing it to two-valued breaks the
    /// feature in a way no test in `scoring` or `selection` would catch, which
    /// is why the rule lives here, once, instead of at each call site.
    pub fn state(&self, facet: &FacetRef) -> FacetState {
        match facet.kind {
            FacetKind::Category => total(self.category == facet.value),
            FacetKind::Chain => total(self.chain == facet.value),
            FacetKind::Tag => {
                if self.tags.contains(&facet.value) {
                    FacetState::Present
                } else {
                    FacetState::Unknown
                }
            }
        }
    }
}

/// Non-match on a total facet, where the schema guarantees exactly one value
/// per row, so "not this one" really is evidence of absence.
fn total(matches: bool) -> FacetState {
    if matches {
        FacetState::Present
    } else {
        FacetState::Absent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> Candidate {
        Candidate {
            app_id: "app1".into(),
            category: "defi".into(),
            chain: "solana".into(),
            tags: HashSet::from(["dex".to_string()]),
            content_score: 0.5,
            support: 0.0,
            outcome_mean: 0.0,
        }
    }

    #[test]
    fn facet_ref_wire_spelling() {
        let facet = FacetRef {
            kind: FacetKind::Tag,
            value: "lending".into(),
        };
        assert_eq!(
            serde_json::to_string(&facet).unwrap(),
            r#"{"kind":"tag","value":"lending"}"#
        );
    }

    #[test]
    fn answer_value_wire_spelling() {
        for (value, wire) in [
            (AnswerValue::Yes, r#""yes""#),
            (AnswerValue::No, r#""no""#),
            (AnswerValue::Skip, r#""skip""#),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), wire);
            assert_eq!(serde_json::from_str::<AnswerValue>(wire).unwrap(), value);
        }
    }

    #[test]
    fn answer_round_trips() {
        let wire = r#"{"facet":{"kind":"category","value":"defi"},"value":"yes"}"#;
        let answer: Answer = serde_json::from_str(wire).unwrap();
        assert_eq!(answer.facet.kind, FacetKind::Category);
        assert_eq!(answer.facet.value, "defi");
        assert_eq!(answer.value, AnswerValue::Yes);
        assert_eq!(serde_json::to_string(&answer).unwrap(), wire);
    }

    #[test]
    fn category_is_total() {
        let candidate = candidate();
        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Category,
                value: "defi".into()
            }),
            FacetState::Present
        );
        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Category,
                value: "nft".into()
            }),
            FacetState::Absent
        );
    }

    #[test]
    fn chain_is_total() {
        let candidate = candidate();
        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Chain,
                value: "solana".into()
            }),
            FacetState::Present
        );
        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Chain,
                value: "base".into()
            }),
            FacetState::Absent
        );
    }

    /// Exists because A14/A21 turn on this one distinction: a tag an app does
    /// not carry is `Unknown`, never `Absent`. If someone "simplifies" the
    /// three-valued state to a boolean, this is the test that catches it —
    /// nothing in `scoring` or `selection` would.
    #[test]
    fn missing_tag_is_unknown_not_absent() {
        let candidate = candidate();
        let lending = FacetRef {
            kind: FacetKind::Tag,
            value: "lending".into(),
        };
        assert_eq!(candidate.state(&lending), FacetState::Unknown);
        assert_ne!(candidate.state(&lending), FacetState::Absent);
        assert_eq!(
            candidate.state(&FacetRef {
                kind: FacetKind::Tag,
                value: "dex".into()
            }),
            FacetState::Present
        );
    }

    /// The no-flip precondition. `blend.rs` unit-tests the full invariant; this
    /// guards the constants themselves, so retuning `LAMBDA` upward past the
    /// content gap fails here rather than silently making farming profitable.
    #[test]
    fn lambda_stays_below_min_content_gap() {
        // Bound to locals so the comparison is not a constant expression:
        // clippy's assertions_on_constants would otherwise reject it as
        // const-folded, and the point here is to fail the build on a retune,
        // not to assert something the compiler already knows.
        let lambda = params::LAMBDA;
        let min_content_gap = params::MIN_CONTENT_GAP;
        assert!(
            lambda < min_content_gap,
            "LAMBDA ({lambda}) must stay strictly below MIN_CONTENT_GAP ({min_content_gap}) \
             or blend.rs's no-flip property is false and farming becomes profitable"
        );
    }
}
