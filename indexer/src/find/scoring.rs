//! Pure log-space posterior over candidates, clamped so none is ever eliminated.
//!
//! `ln w_i = ln prior_i + SUM_q damping * ln p(r_q | q, y_i)` — the naive-Bayes
//! accumulation from `research/soft-scoring-update.md`. Nothing here touches
//! `sqlx`, `axum` or `ApiState`; the tests build candidates in memory, which is
//! brief success criterion 5.

use crate::find::params;
use crate::find::{Answer, AnswerValue, Candidate, FacetState};

/// `p(r | q, y_i)` — how likely a user is to answer `r` about facet `q`, given
/// candidate `y_i`'s state for it.
///
/// `Skip` ("don't care") returns exactly `1.0`: the multiplicative identity, a
/// strict no-op rather than a damped update. This is the one point where both
/// documented precedents agree — R's `e1071` omits the table entry entirely, and
/// Burgener's 20Q patent states "Unknown is not counted as an answer" — and no
/// retrieved source endorses a partial update, so inventing one would put an
/// uncited number directly into the term that drives the stopping rule.
///
/// `Unknown` marginalizes over the tag-prevalence prior `pi`, integrating out a
/// latent value we never observed. A crowd-sourced tag nobody has recorded is
/// **not** a "no" (A14) — and this row is the single mechanism that stops
/// well-tagged apps from beating thinly-tagged ones on tag coverage alone.
/// Collapsing it onto the `Absent` row breaks the feature quietly.
///
/// Evidential values are clamped to `[EPS, 1 - EPS]`, so no likelihood is ever
/// zero, no candidate is ever eliminated, and one wrong answer costs a bounded
/// `ln((1 - EPS) / EPS)` that later answers can outvote. That is what makes the
/// shortlist non-empty on every answer path — including self-contradictory ones
/// — by construction rather than by luck.
pub fn answer_likelihood(state: FacetState, answer: AnswerValue) -> f64 {
    let e = params::EPS;
    let pi = params::TAG_PREVALENCE_PRIOR;
    let p = match (state, answer) {
        (_, AnswerValue::Skip) => return 1.0,
        (FacetState::Present, AnswerValue::Yes) => 1.0 - e,
        (FacetState::Present, AnswerValue::No) => e,
        (FacetState::Absent, AnswerValue::Yes) => e,
        (FacetState::Absent, AnswerValue::No) => 1.0 - e,
        (FacetState::Unknown, AnswerValue::Yes) => pi * (1.0 - e) + (1.0 - pi) * e,
        (FacetState::Unknown, AnswerValue::No) => pi * e + (1.0 - pi) * (1.0 - e),
    };
    p.clamp(e, 1.0 - e)
}

/// `ln w_i`, unnormalized. See the module doc for the formula.
///
/// **The `CORRELATION_DAMPING` factor is the A13 mitigation and is not
/// optional.** Conditional independence across answers is *known false* for this
/// facet set — an app tagged `lending` is almost certainly category `defi` — so
/// correlated evidence is double-counted and over-sharpens the posterior. Naive
/// Bayes normally shrugs that off because `argmax` survives miscalibration, but
/// here the posterior drives the **stopping rule** (A12), where an over-sharp
/// posterior makes the funnel stop early and over-confident. Damping the
/// exponent below 1 slows the sharpening, so the threshold fires later rather
/// than wrongly. The two alternative mitigations — dropping facets correlated
/// with an already-answered one, or calibrating the threshold directly — both
/// need correlation estimates from session data that does not exist yet.
///
/// `prior_i` is the candidate's content score floored at `params::EPS`. The
/// floor is load-bearing rather than defensive: `content_score` is min-max
/// normalized, so the worst-ranked app sits at exactly `0.0` and `ln 0` would
/// hand it `-inf` — eliminating it, which is precisely what the clamp above
/// exists to prevent. `EPS` is reused rather than introducing a second constant
/// because it already means "the floor below which nothing in this engine is
/// allowed to fall".
pub fn log_weight(candidate: &Candidate, answers: &[Answer]) -> f64 {
    let prior = candidate.content_score.max(params::EPS);
    answers.iter().fold(prior.ln(), |acc, answer| {
        let likelihood = answer_likelihood(candidate.state(&answer.facet), answer.value);
        acc + params::CORRELATION_DAMPING * likelihood.ln()
    })
}

/// The normalized posterior over `candidates`, in input order, summing to 1.
///
/// Exponentiation subtracts the maximum log-weight first — standard log-sum-exp
/// hygiene, and at today's parameters that is *all* it is. The floor on a log
/// weight is `ln(EPS) + MAX_QUESTIONS * CORRELATION_DAMPING * ln(EPS)` ≈ -19.8,
/// i.e. `exp` ≈ 2.6e-9: nowhere near `f64` underflow, which needs roughly 355
/// answers. The subtraction earns its place by staying correct if `EPS` is
/// driven far smaller or `MAX_QUESTIONS` far larger, not by rescuing the shipped
/// configuration — no test here fails without it.
pub fn posterior(candidates: &[Candidate], answers: &[Answer]) -> Vec<f64> {
    let logs: Vec<f64> = candidates.iter().map(|c| log_weight(c, answers)).collect();
    let max = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let weights: Vec<f64> = logs.iter().map(|l| (l - max).exp()).collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        // Only reachable when `candidates` is empty: the max-subtract guarantees
        // at least one weight is exactly `exp(0) == 1`.
        return vec![0.0; candidates.len()];
    }
    weights.into_iter().map(|w| w / total).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::{FacetKind, FacetRef};

    fn candidate(app_id: &str, category: &str, tags: &[&str], content_score: f64) -> Candidate {
        Candidate {
            app_id: app_id.into(),
            category: category.into(),
            chain: "solana".into(),
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            content_score,
            support: 0.0,
            outcome_mean: 0.0,
        }
    }

    fn tag(value: &str) -> FacetRef {
        FacetRef {
            kind: FacetKind::Tag,
            value: value.into(),
        }
    }

    fn category(value: &str) -> FacetRef {
        FacetRef {
            kind: FacetKind::Category,
            value: value.into(),
        }
    }

    fn answered(facet: FacetRef, value: AnswerValue) -> Answer {
        Answer { facet, value }
    }

    const STATES: [FacetState; 3] = [FacetState::Present, FacetState::Absent, FacetState::Unknown];

    #[test]
    fn present_state_answers_at_the_clamp_bounds() {
        assert_eq!(
            answer_likelihood(FacetState::Present, AnswerValue::Yes),
            1.0 - params::EPS
        );
        assert_eq!(
            answer_likelihood(FacetState::Present, AnswerValue::No),
            params::EPS
        );
    }

    #[test]
    fn absent_state_inverts_present() {
        assert_eq!(
            answer_likelihood(FacetState::Absent, AnswerValue::Yes),
            params::EPS
        );
        assert_eq!(
            answer_likelihood(FacetState::Absent, AnswerValue::No),
            1.0 - params::EPS
        );
    }

    /// `Skip` is a strict no-op — the multiplicative identity — for every state,
    /// not a damped update. Both documented precedents agree and no source
    /// endorses a partial update.
    #[test]
    fn skip_is_a_strict_no_op_for_every_state() {
        for state in STATES {
            assert_eq!(answer_likelihood(state, AnswerValue::Skip), 1.0);
        }
    }

    /// A14: `Unknown` marginalizes over the tag-prevalence prior, so it sits
    /// strictly between `Present` and `Absent` and is **not** equal to `Absent`.
    /// This test exists to fail loudly if someone "simplifies" the three-valued
    /// state to a boolean, which would silently make a missing crowd-sourced tag
    /// read as a "no" and let well-tagged apps dominate thin ones.
    #[test]
    fn unknown_is_not_absent_a14() {
        let present = answer_likelihood(FacetState::Present, AnswerValue::Yes);
        let absent = answer_likelihood(FacetState::Absent, AnswerValue::Yes);
        let unknown = answer_likelihood(FacetState::Unknown, AnswerValue::Yes);

        assert!(
            unknown > absent,
            "unknown {unknown} must exceed absent {absent}"
        );
        assert!(
            unknown < present,
            "unknown {unknown} must be below present {present}"
        );
        assert_ne!(unknown, absent);

        let expected = params::TAG_PREVALENCE_PRIOR * (1.0 - params::EPS)
            + (1.0 - params::TAG_PREVALENCE_PRIOR) * params::EPS;
        assert!((unknown - expected).abs() < 1e-12);
    }

    /// The clamp is what makes "no candidate is ever eliminated" true by
    /// construction. `Skip` is the deliberate exception: it is not evidence at
    /// all, so it returns the identity `1.0` rather than a bounded probability.
    #[test]
    fn evidential_likelihoods_stay_inside_the_clamp() {
        for state in STATES {
            for answer in [AnswerValue::Yes, AnswerValue::No, AnswerValue::Skip] {
                let l = answer_likelihood(state, answer);
                assert!(l >= params::EPS, "{state:?}/{answer:?} = {l} below EPS");
                assert!(l <= 1.0, "{state:?}/{answer:?} = {l} above 1.0");
                if answer != AnswerValue::Skip {
                    assert!(
                        l <= 1.0 - params::EPS,
                        "{state:?}/{answer:?} = {l} above 1 - EPS"
                    );
                }
            }
        }
    }

    #[test]
    fn posterior_normalizes_and_preserves_order() {
        // Equal priors, so the tag evidence alone decides: "a" and "c" carry it,
        // "b" does not. The odd one out landing at index 1 is what shows the
        // output is aligned with the input, not sorted by score.
        let candidates = [
            candidate("a", "defi", &["lending"], 0.5),
            candidate("b", "nft", &[], 0.5),
            candidate("c", "gaming", &["lending"], 0.5),
        ];
        let post = posterior(&candidates, &[answered(tag("lending"), AnswerValue::Yes)]);

        assert_eq!(post.len(), 3);
        assert!((post.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!((post[0] - post[2]).abs() < 1e-12);
        assert!(post[1] < post[0]);

        let reversed: Vec<Candidate> = candidates.iter().rev().cloned().collect();
        let mirrored = posterior(&reversed, &[answered(tag("lending"), AnswerValue::Yes)]);
        for i in 0..3 {
            assert!((mirrored[i] - post[2 - i]).abs() < 1e-12);
        }
    }

    /// Brief success criterion 2: the shortlist is never empty, for any answer
    /// path including self-contradictory ones.
    #[test]
    fn no_candidate_is_ever_eliminated() {
        let candidates = [
            candidate("a", "defi", &["lending"], 1.0),
            candidate("b", "nft", &[], 0.0),
            candidate("c", "gaming", &["dex"], 0.5),
        ];
        let answers = [
            answered(tag("lending"), AnswerValue::Yes),
            answered(tag("lending"), AnswerValue::No),
            answered(category("defi"), AnswerValue::Yes),
            answered(category("nft"), AnswerValue::Yes),
        ];
        let post = posterior(&candidates, &answers);

        assert!((post.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        for (i, p) in post.iter().enumerate() {
            assert!(*p > 0.0, "candidate {i} was eliminated (p = {p})");
            assert!(p.is_finite());
        }
    }

    #[test]
    fn all_skip_leaves_the_prior_untouched() {
        let candidates = [
            candidate("a", "defi", &["lending"], 0.8),
            candidate("b", "nft", &[], 0.2),
            candidate("c", "gaming", &[], 0.0),
        ];
        let skips = [
            answered(tag("lending"), AnswerValue::Skip),
            answered(category("defi"), AnswerValue::Skip),
            answered(category("nft"), AnswerValue::Skip),
        ];
        let updated = posterior(&candidates, &skips);
        let untouched = posterior(&candidates, &[]);

        let floored: Vec<f64> = candidates
            .iter()
            .map(|c| c.content_score.max(params::EPS))
            .collect();
        let total: f64 = floored.iter().sum();

        for i in 0..candidates.len() {
            assert!((updated[i] - untouched[i]).abs() < 1e-12);
            assert!((updated[i] - floored[i] / total).abs() < 1e-12);
        }
    }

    #[test]
    fn a_matching_candidate_gains_mass_over_an_identical_non_match() {
        let candidates = [
            candidate("match", "defi", &["lending"], 0.5),
            candidate("miss", "nft", &["lending"], 0.5),
        ];
        let before = posterior(&candidates, &[]);
        assert!((before[0] - before[1]).abs() < 1e-12);

        let after = posterior(&candidates, &[answered(category("defi"), AnswerValue::Yes)]);
        assert!(
            after[0] > after[1],
            "matching candidate {} did not outrank {}",
            after[0],
            after[1]
        );
    }

    /// A13/A27: the damping factor must actually be in the exponent. Asserted
    /// against the arithmetic rather than a remembered number, so retuning
    /// `CORRELATION_DAMPING` does not require editing this test.
    #[test]
    fn correlation_damping_is_applied_to_every_term() {
        let c = candidate("a", "defi", &["lending"], 0.5);
        let answers = [
            answered(tag("lending"), AnswerValue::Yes),
            answered(category("nft"), AnswerValue::Yes),
        ];

        let prior_ln = c.content_score.max(params::EPS).ln();
        let undamped: f64 = answers
            .iter()
            .map(|a| answer_likelihood(c.state(&a.facet), a.value).ln())
            .sum();
        assert!(undamped.abs() > 0.0, "test needs non-trivial evidence");

        let actual = log_weight(&c, &answers);
        assert!((actual - (prior_ln + params::CORRELATION_DAMPING * undamped)).abs() < 1e-12);
        assert!(
            (actual - prior_ln).abs() < undamped.abs(),
            "damped evidence {} must be strictly closer to the prior than undamped {}",
            actual - prior_ln,
            undamped
        );
    }
}
