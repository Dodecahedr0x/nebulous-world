//! Bounded-influence blend: shrinkage plus the hard cap that makes farming unprofitable.
//!
//! Surfacing in `/find` drives traffic, traffic drives ad revenue, and ad
//! revenue accrues to stakers — so farming confirmations on your own app is
//! directly profitable. Two separate mechanisms answer that, and **neither can
//! do the other's job** (A16). They are two functions here for exactly that
//! reason; folding them into one hides which half is load-bearing against an
//! adversary.
//!
//! 1. [`shrunk_learned`] — `(v/(v+m))*R + (m/(v+m))*C`, the Bayesian-average
//!    form. It handles **cold start and small-sample noise only**. It does not
//!    bound an adversary: `v` is attacker-controlled, `v/(v+m) -> 1`, so a
//!    patient farmer simply buys their way to the unshrunk point estimate.
//!    *Rejected alternative:* the Wilson lower bound, for the identical reason
//!    — its LCB converges to `p̂` as `n` grows, so it corrects small samples,
//!    not attackers.
//! 2. [`blend`] — `final = content + LAMBDA * learned`, with `learned` in
//!    `[0, 1]` and `LAMBDA` strictly below `MIN_CONTENT_GAP`. This is the part
//!    that makes farming unprofitable *at any volume*, because the whole
//!    learned term is capped at `LAMBDA` no matter how large `v` gets.
//!
//! The cap has a named ancestor — Resnick & Sami's influence limiter (RecSys
//! 2007) — but the retrieved 2024 poisoning surveys taxonomize defences as data
//! *filtering* plus *robust training* and recommend detection, not static caps,
//! and no retrieved source measures prediction-shift under a capped additive
//! term in a content+CF hybrid. So the no-flip property below is **algebra we
//! assert and unit-test**, not a borrowed guarantee. Describe this design as a
//! "bounded-influence blend, plus Turnstile and rate limiting" — never as "the
//! standard anti-shilling defence" (A18). Overstating the pedigree would
//! mislead whoever tunes this next.
//!
//! This module is **pure**: `std` and [`crate::find::params`] only. No `sqlx`,
//! no `axum`, no `async`, no IO — the maths is unit-testable with no database,
//! per the repo root `AGENTS.md`.

use crate::find::params::{
    LAMBDA, NEUTRAL_OUTCOME_C, SHRINKAGE_M, WEIGHT_CLICKED, WEIGHT_CONFIRMED, WEIGHT_REJECTED,
};

/// Confidence-weighted learned term, clamped to `[0, 1]`.
///
/// `(v / (v + m)) * R  +  (m / (v + m)) * C`
///
/// `support` is `v`, the count of completed-session outcomes naming this app;
/// `outcome_mean` is `R`, already normalized into `[0, 1]` by
/// [`normalize_outcome_mean`].
///
/// Handles cold start and small-sample noise **only**. It does not bound an
/// adversary — `v` is attacker-controlled and the shrinkage weight tends to 1.
/// [`blend`]'s cap is what does that job.
pub fn shrunk_learned(support: f64, outcome_mean: f64) -> f64 {
    // A negative, NaN or infinite `v` is corrupt data, not a signal. Coercing it
    // to zero support lands on the neutral prior, which is the fail-safe
    // direction: garbage can only ever cost an app its learned lift, never grant
    // one. Letting a NaN through would be far worse than wrong — it poisons
    // every subsequent comparison and silently reorders the shortlist, because
    // NaN is unordered against everything.
    let v = if support.is_finite() && support > 0.0 {
        support
    } else {
        0.0
    };
    let r = clamp_unit(outcome_mean, NEUTRAL_OUTCOME_C);

    // Only reachable if someone retunes SHRINKAGE_M to zero and v is zero; the
    // guard exists so that retune produces a neutral term rather than 0/0 NaN.
    let total = v + SHRINKAGE_M;
    if total <= 0.0 {
        return clamp_unit(NEUTRAL_OUTCOME_C, 0.0);
    }

    let weighted = (v / total) * r + (SHRINKAGE_M / total) * NEUTRAL_OUTCOME_C;
    clamp_unit(weighted, 0.0)
}

/// Clamps into `[0, 1]`, substituting `fallback` for a non-finite input.
/// `f64::clamp` alone is not enough: it returns NaN for a NaN input.
fn clamp_unit(value: f64, fallback: f64) -> f64 {
    let value = if value.is_finite() { value } else { fallback };
    value.clamp(0.0, 1.0)
}

/// `final = content + LAMBDA * shrunk_learned(support, outcome_mean)`.
///
/// `content` is the answer-conditioned posterior in `[0, 1]` (A32), not the raw
/// quality score.
///
/// `content` is deliberately **not** sanitized, unlike the learned inputs: it is
/// the caller's contract, and a NaN posterior is a bug in the scorer that
/// silently clamping to 0 would bury under a plausible-looking score.
pub fn blend(content: f64, support: f64, outcome_mean: f64) -> f64 {
    content + LAMBDA * shrunk_learned(support, outcome_mean)
}

/// The no-flip guarantee, stated as the algebra it is.
///
/// When A's content score exceeds B's by more than `LAMBDA`, no amount of
/// learned signal on B can put it above A, because the learned term contributes
/// at most `LAMBDA * 1.0`. Returns true when that precondition holds for the
/// pair.
pub fn preserves_order(content_a: f64, content_b: f64) -> bool {
    // Strict: at a gap of exactly LAMBDA a fully-farmed B ties A rather than
    // losing to it, and a tie is not an order we are willing to promise.
    content_a - content_b > LAMBDA
}

/// Maps a stored outcome string to its scoring weight. `None` for anything
/// unrecognised — the caller decides whether that is a 400 or a skipped row.
/// Matching is exact and case-sensitive: these are the three wire spellings of
/// `POST /find/confirm`'s `outcome` (A19), not free text.
pub fn outcome_weight(outcome: &str) -> Option<f64> {
    match outcome {
        "confirmed" => Some(WEIGHT_CONFIRMED),
        "clicked" => Some(WEIGHT_CLICKED),
        "rejected" => Some(WEIGHT_REJECTED),
        _ => None,
    }
}

/// Normalizes a mean outcome weight into the `[0, 1]` range [`shrunk_learned`]
/// expects: `(w - WEIGHT_REJECTED) / (WEIGHT_CONFIRMED - WEIGHT_REJECTED)`,
/// clamped.
///
/// `WEIGHT_REJECTED` is negative, so the raw mean weight lives on a signed
/// scale; `shrunk_learned` needs `[0, 1]`. Rescaling here rather than inside
/// `shrunk_learned` keeps the shrinkage formula the textbook one.
pub fn normalize_outcome_mean(mean_weight: f64) -> f64 {
    let span = WEIGHT_CONFIRMED - WEIGHT_REJECTED;
    if span <= 0.0 {
        return 0.0;
    }
    clamp_unit((mean_weight - WEIGHT_REJECTED) / span, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::params::MIN_CONTENT_GAP;

    /// Every support count the no-flip property is swept over. Spread across
    /// nine orders of magnitude so the test is about the bound, not about one
    /// lucky number.
    const SUPPORT_SWEEP: [f64; 8] = [
        1.0,
        10.0,
        100.0,
        1_000.0,
        10_000.0,
        100_000.0,
        1_000_000.0,
        1e9,
    ];

    #[test]
    fn zero_support_contributes_nothing() {
        assert_eq!(shrunk_learned(0.0, 1.0), 0.0);
    }

    #[test]
    fn shrunk_learned_is_monotonic_and_bounded_in_support() {
        let mut previous = shrunk_learned(0.0, 1.0);
        for support in [0.0, 1.0, 10.0, 100.0, 10_000.0, 1e9] {
            let value = shrunk_learned(support, 1.0);
            assert!(
                (0.0..=1.0).contains(&value),
                "shrunk_learned({support}, 1.0) = {value} escaped [0, 1]"
            );
            assert!(
                value >= previous,
                "shrunk_learned must not decrease as support grows: {previous} -> {value} at v={support}"
            );
            previous = value;
        }
        // The convergence that is *precisely why* shrinkage cannot stand in for
        // the cap: at large `v` the shrunk estimate is the raw point estimate,
        // and `v` is the attacker's to choose (A16).
        assert!(
            previous > 0.99,
            "shrinkage should converge to R as v grows; got {previous} at v=1e9"
        );
    }

    /// The defining property of the Bayesian-average form: at `v == m` the
    /// estimate sits exactly halfway between the prior and the observation.
    #[test]
    fn support_equal_to_m_is_the_midpoint_of_c_and_r() {
        let r = 0.8;
        let expected = 0.5 * r + 0.5 * NEUTRAL_OUTCOME_C;
        assert!((shrunk_learned(SHRINKAGE_M, r) - expected).abs() < 1e-12);
    }

    #[test]
    fn blend_with_no_support_is_exactly_the_content_score() {
        for content in [0.0, 0.2, 0.5, 1.0] {
            assert_eq!(blend(content, 0.0, 1.0), content);
        }
    }

    /// The cap's precondition, asserted here so a future tuning pass fails the
    /// build rather than silently making farming profitable. Bound to locals
    /// because clippy's `assertions_on_constants` const-folds the direct form —
    /// the point is to fail on a retune, not to assert what the compiler knows.
    #[test]
    fn lambda_stays_strictly_below_min_content_gap() {
        let lambda = LAMBDA;
        let min_content_gap = MIN_CONTENT_GAP;
        assert!(
            lambda < min_content_gap,
            "LAMBDA ({lambda}) must stay strictly below MIN_CONTENT_GAP ({min_content_gap}) \
             or the no-flip property below is false"
        );
    }

    /// **Brief success criterion 4.** A deliberately bad match, with its learned
    /// term driven to the theoretical maximum a farmer could ever reach, still
    /// ranks strictly below a well-matched app that has never been seen before.
    /// This is the algebra we assert rather than a result we cite (A16).
    #[test]
    fn farming_cannot_lift_a_bad_match_at_any_n() {
        let bad_content = 0.20;
        let good_content = bad_content + MIN_CONTENT_GAP;

        let farmed = blend(bad_content, 1e9, 1.0);
        let honest = blend(good_content, 0.0, 0.0);

        // The farming must actually buy the attacker something, or this test
        // would also pass against a blend that ignores the learned term
        // entirely — proving nothing about the cap.
        assert!(
            farmed > bad_content,
            "farming bought nothing at all ({farmed} vs {bad_content}); the learned term is dead, \
             so this test is not exercising the cap"
        );
        assert!(
            farmed < honest,
            "a maximally farmed bad match ({farmed}) reached a well-matched app ({honest}) — \
             the cap is not holding"
        );
    }

    /// The same property at every N in the sweep, so the guarantee is about the
    /// bound rather than about one chosen support count.
    #[test]
    fn no_flip_holds_at_every_support_count() {
        let bad_content = 0.20;
        let good_content = bad_content + MIN_CONTENT_GAP;
        let honest = blend(good_content, 0.0, 0.0);

        for support in SUPPORT_SWEEP {
            let farmed = blend(bad_content, support, 1.0);
            assert!(
                farmed > bad_content,
                "farming {support} confirmations bought nothing; the learned term is dead and \
                 this sweep proves nothing about the cap"
            );
            assert!(
                farmed < honest,
                "farming {support} confirmations lifted the bad match to {farmed}, at or above \
                 the honest {honest}"
            );
        }
    }

    #[test]
    fn preserves_order_is_true_exactly_above_lambda() {
        assert!(preserves_order(0.5, 0.4));
        assert!(preserves_order(MIN_CONTENT_GAP, 0.0));
        assert!(preserves_order(LAMBDA * 2.0, 0.0));

        // Exactly at the boundary the precondition is strict, so this is false.
        assert!(!preserves_order(LAMBDA, 0.0));
        assert!(!preserves_order(0.5, 0.5));
        assert!(!preserves_order(0.4, 0.5));
    }

    #[test]
    fn preserves_order_implies_the_order_survives_any_support() {
        for (a, b) in [(0.5, 0.4), (1.0, 0.0), (MIN_CONTENT_GAP, 0.0), (0.9, 0.85)] {
            assert!(
                preserves_order(a, b),
                "test fixture ({a}, {b}) is not above LAMBDA"
            );
            for support in SUPPORT_SWEEP {
                assert!(
                    blend(a, 0.0, 0.0) > blend(b, support, 1.0),
                    "order ({a} over {b}) flipped at support {support}"
                );
            }
        }
    }

    #[test]
    fn outcome_weight_maps_known_strings_only() {
        assert_eq!(outcome_weight("confirmed"), Some(WEIGHT_CONFIRMED));
        assert_eq!(outcome_weight("clicked"), Some(WEIGHT_CLICKED));
        assert_eq!(outcome_weight("rejected"), Some(WEIGHT_REJECTED));
        assert_eq!(outcome_weight("upvoted"), None);
        assert_eq!(outcome_weight(""), None);
        assert_eq!(outcome_weight("Confirmed"), None);
    }

    #[test]
    fn normalize_outcome_mean_anchors_the_scale_and_clamps() {
        assert_eq!(normalize_outcome_mean(WEIGHT_REJECTED), 0.0);
        assert_eq!(normalize_outcome_mean(WEIGHT_CONFIRMED), 1.0);
        assert!((normalize_outcome_mean(0.0) - 0.5).abs() < 1e-12);

        for outside in [-100.0, 100.0, f64::NEG_INFINITY, f64::INFINITY, f64::NAN] {
            let value = normalize_outcome_mean(outside);
            assert!(
                (0.0..=1.0).contains(&value),
                "normalize_outcome_mean({outside}) = {value} escaped [0, 1]"
            );
        }
    }

    /// Corrupt support must never produce NaN: a NaN score poisons every
    /// comparison downstream and would silently reorder the shortlist.
    #[test]
    fn degenerate_inputs_stay_finite_and_bounded() {
        for support in [f64::NAN, -5.0, f64::INFINITY, f64::NEG_INFINITY] {
            let value = shrunk_learned(support, 0.5);
            assert!(
                value.is_finite() && (0.0..=1.0).contains(&value),
                "shrunk_learned({support}, 0.5) = {value}"
            );
        }
        for mean in [f64::NAN, -3.0, 4.0, f64::INFINITY] {
            let value = shrunk_learned(100.0, mean);
            assert!(
                value.is_finite() && (0.0..=1.0).contains(&value),
                "shrunk_learned(100.0, {mean}) = {value}"
            );
            assert!(blend(0.5, 100.0, mean).is_finite());
        }
    }
}
