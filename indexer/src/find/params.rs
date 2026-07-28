//! Every tunable of the `/find` funnel, in one place, so retuning is a
//! one-file diff rather than a hunt through the engine.
//!
//! **None of these has a citable correct value** (A7, A17) — both Phase 2
//! research files say so explicitly. They are starting points to be tuned on
//! held-out sessions once sessions exist; today there are zero. Each doc
//! comment therefore records *what evidence would change it*, not a
//! justification for the number. Magic numbers do not belong anywhere else in
//! `find/`.

/// Hard cap on questions per session. Lower it if measured questions-to-confirm
/// clusters well below the cap; raise it only if sessions routinely hit the cap
/// without the posterior threshold firing.
pub const MAX_QUESTIONS: usize = 8;

/// Stopping threshold `delta`: stop once `p_top1 >= 1 - delta` (A12). Raise if
/// sessions run long and users bail; lower if confirmed-outcome rate on the
/// top suggestion is poor, which means we are stopping over-confident.
pub const POSTERIOR_STOP_DELTA: f64 = 0.15;

/// Alternative stop: normalized posterior entropy below this floor. Guards the
/// flat-distribution case where no single candidate crosses the threshold but
/// the field has genuinely collapsed. Tune against the same session data as
/// `POSTERIOR_STOP_DELTA`.
pub const ENTROPY_STOP_FLOOR: f64 = 0.25;

/// The likelihood clamp. Every likelihood is clamped to `[EPS, 1 - EPS]` so no
/// candidate is ever eliminated by a single answer and one wrong answer costs a
/// bounded `ln((1 - EPS) / EPS)` that later answers can outvote. Smaller EPS
/// means sharper updates and less forgiveness of a misclick; larger means the
/// funnel needs more questions to separate anything. Change it on evidence
/// about how often users answer wrongly, not on how the posterior looks.
pub const EPS: f64 = 0.05;

/// Prior probability that an app carries a given tag, used to marginalize the
/// `FacetState::Unknown` case instead of treating a missing tag as evidence of
/// absence (A14). A single global value, not a per-tag estimate — which is why
/// there is deliberately no additive smoothing constant here (A50): with no
/// per-tag counts, there is nothing to smooth. Reintroduce both together if
/// `AppTag` coverage is ever measured well enough to estimate prevalence
/// per tag.
pub const TAG_PREVALENCE_PRIOR: f64 = 0.10;

/// The A13 mitigation. Conditional independence across answers is **known
/// false** for this facet set — an app tagged `lending` is almost certainly
/// category `defi` — so correlated evidence gets double-counted and
/// over-sharpens the posterior. That matters precisely because the posterior
/// drives the stopping rule (A12): an over-sharp posterior stops the funnel
/// early and over-confident. The chosen mitigation is to damp every
/// log-likelihood by this exponent `< 1`. Calibrate it against observed
/// confirm rates at the moment of stopping — if the funnel stops confident and
/// is wrong, damp harder.
pub const CORRELATION_DAMPING: f64 = 0.7;

/// Floor below which a facet's expected information gain counts as zero, so a
/// numerically-noisy near-tie does not get asked as if it discriminated. Purely
/// a numerical guard; change only if the IG computation's scale changes.
pub const MIN_INFORMATION_GAIN: f64 = 1e-6;

/// A tag must be carried by at least this many apps to be worth asking about —
/// a tag on one app splits the field almost not at all and reads as a trivia
/// question. Raise it if question quality complaints centre on obscure tags.
pub const MIN_TAG_SUPPORT: usize = 3;

/// The shrinkage constant `m` in `(v/(v+m))*R + (m/(v+m))*C`. Handles cold
/// start and small-sample noise **only** — it does not bound an adversary,
/// because `v` is attacker-controlled and `v/(v+m) -> 1` (A16). Algolia's
/// documented heuristic is "lower quartile of per-item support"; apply it once
/// there is support data to compute a quartile from.
pub const SHRINKAGE_M: f64 = 20.0;

/// The shrinkage prior `C`. Zero so that a candidate with `support == 0`
/// contributes exactly nothing to the learned term and the funnel is pure
/// content score at cold start — which is today's state, with zero logged
/// sessions. Only move it off zero if an app with no history should be
/// optimistically or pessimistically biased relative to one with a mediocre
/// record, which is a product call, not a tuning one.
pub const NEUTRAL_OUTCOME_C: f64 = 0.0;

/// The hard cap on the learned term: `final = content + LAMBDA * learned`, with
/// `learned` in `[0, 1]`. **Must stay strictly less than `MIN_CONTENT_GAP`** —
/// `blend.rs` unit-tests that relationship, and it is what makes farming
/// confirmations unprofitable at any volume N, since even a maximally-farmed
/// learned term cannot close a content gap. This is a bounded-influence blend,
/// plus Turnstile and rate limiting on the confirm endpoint; it is not "the
/// standard anti-shilling defence" (A18) — the retrieved poisoning surveys
/// recommend detection and robust training, and overstating the pedigree here
/// would mislead the next reader. Raise it only together with evidence that the
/// learned signal beats content score, and never above `MIN_CONTENT_GAP`.
pub const LAMBDA: f64 = 0.02;

/// The posterior gap a genuinely good match is expected to open over a
/// genuinely bad one (A32). The `content` side of the blend is the
/// answer-conditioned normalized posterior, not the raw quality score, and
/// posteriors are continuous — so this is **not** a claim that the scorer emits
/// values at least this far apart. It is the scale `LAMBDA` is chosen against.
/// The property that actually holds is algebra, and it is what `blend.rs`
/// tests: `content_a - content_b > LAMBDA` implies the order survives any
/// support count whatsoever. Revisit this number if measured posteriors at the
/// stopping point routinely separate by less.
pub const MIN_CONTENT_GAP: f64 = 0.05;

/// Outcome weight for an explicit "yes, this is the one" confirm — the
/// strongest training signal the funnel collects, so it anchors the scale at 1.
pub const WEIGHT_CONFIRMED: f64 = 1.0;

/// Outcome weight for a click-through without a confirm: interest, but not
/// satisfaction. Tune from the measured correlation between clicks and later
/// confirms; if they turn out to track closely, raise it toward 1.
pub const WEIGHT_CLICKED: f64 = 0.2;

/// Outcome weight for an explicit "not quite" (A9). Negative so a rejection
/// actively demotes the app on that answer path rather than merely failing to
/// promote it. Make it less negative if a single grumpy session visibly buries
/// an otherwise good match.
pub const WEIGHT_REJECTED: f64 = -1.0;

/// Maximum apps returned in the final shortlist (A20). This is a **leak
/// control**, not a UI preference: the shortlist is the only place `/find`
/// discloses app identities, so it stays bounded and only ships at `done`.
/// Raising it widens what a caller can enumerate by sweeping answers.
pub const SHORTLIST_LIMIT: usize = 5;
