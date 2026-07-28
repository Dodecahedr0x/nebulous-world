//! Pure question choice by expected information gain, plus the stopping rule.
//!
//! The criterion is mutual information over the normalized candidate
//! distribution, **not** "the question that splits the set closest to 50/50"
//! (A11). Those two are equivalent only when the answer is a deterministic
//! function of the candidate; ours are deliberately soft, and Golovin & Krause
//! show generalized binary search "can perform very poorly" under noise. The
//! `max_ig_is_not_the_even_split_rule_a11` test below exists to keep it that
//! way. Greedy one-step selection is defensible on Jedynak/Frazier/Sznitman —
//! support, not proof, since their result covers arbitrary-subset questions
//! rather than a fixed facet list.

use crate::find::{params, scoring, Answer, AnswerValue, Candidate, FacetKind, FacetRef};

/// Shannon entropy in nats. `0.0` for an empty or degenerate distribution;
/// zero-probability terms are skipped rather than evaluated, since `0 * ln 0` is
/// `NaN` in IEEE arithmetic and one `NaN` would poison every comparison
/// downstream.
pub fn entropy(p: &[f64]) -> f64 {
    -p.iter()
        .filter(|x| **x > 0.0)
        .map(|x| x * x.ln())
        .sum::<f64>()
}

/// Expected information gain of asking about `facet`, given the current
/// posterior `post` over `candidates` (same order, as `scoring::posterior`
/// returns it):
///
/// ```text
/// p(r|X)   = SUM_i post_i * p(r | q, y_i)
/// H(y|X,q) = SUM_r p(r|X) * H(y | X, q, r)
/// IG(q)    = H(y|X) - H(y|X,q)
/// ```
///
/// `H(y|X)` comes straight from `post`, so a caller that already holds the
/// posterior does not pay to recompute it per facet.
///
/// The answer set is `{Yes, No}` only (A33): `Skip` performs no scoring update,
/// so its conditional entropy is exactly the current entropy and it contributes
/// zero information by construction. Including it would scale every facet's IG
/// by the same unknown `p(skip)`, which the engine has no data to estimate. The
/// two retained answers already sum to 1 for every `FacetState` — `Present` and
/// `Absent` give `(1-e) + e`, and `Unknown` gives `pi + (1-pi)` — so no
/// renormalization is needed.
///
/// Likelihoods here are **undamped**, unlike `scoring::log_weight`. The A13
/// damping corrects double-counting of *correlated evidence across answers*;
/// this function scores a single next answer in isolation, where nothing has
/// been counted twice yet.
///
/// The result is floored at zero. Mutual information cannot be negative, so the
/// floor only absorbs float cancellation near a genuine zero.
pub fn expected_information_gain(candidates: &[Candidate], post: &[f64], facet: &FacetRef) -> f64 {
    if candidates.is_empty() || candidates.len() != post.len() {
        return 0.0;
    }

    let mut conditional = 0.0;
    for answer in [AnswerValue::Yes, AnswerValue::No] {
        let likelihoods: Vec<f64> = candidates
            .iter()
            .map(|c| scoring::answer_likelihood(c.state(facet), answer))
            .collect();
        let p_answer: f64 = post.iter().zip(&likelihoods).map(|(w, l)| w * l).sum();
        if p_answer <= 0.0 {
            continue;
        }
        let branch: Vec<f64> = post
            .iter()
            .zip(&likelihoods)
            .map(|(w, l)| w * l / p_answer)
            .collect();
        conditional += p_answer * entropy(&branch);
    }

    (entropy(post) - conditional).max(0.0)
}

/// Probability that `facet` is present on `candidate`, as a soft indicator.
///
/// `Unknown` resolves to the tag-prevalence prior rather than to 0, for the same
/// reason `scoring` marginalizes it: an unrecorded tag is not a recorded
/// absence, and collapsing it to 0 would make every thinly-tagged app look
/// identical to every app that genuinely lacks the tag.
fn presence(candidate: &Candidate, facet: &FacetRef) -> f64 {
    match candidate.state(facet) {
        crate::find::FacetState::Present => 1.0,
        crate::find::FacetState::Absent => 0.0,
        crate::find::FacetState::Unknown => params::TAG_PREVALENCE_PRIOR,
    }
}

/// How much a question about `a` already covers a question about `b`, in
/// `[0, 1]`. `1.0` means asking `b` next is asking the same thing again.
///
/// Two rules, because there are two genuinely different relationships here and
/// one measure cannot see both:
///
/// 1. **Same single-valued field ⇒ fully redundant.** `category` and `chain` are
///    one-of-N columns on the app row: every app has exactly one. So "not
///    Solana?" followed by "not Ethereum?" is one dimension asked twice, however
///    the numbers fall. This is structural and no statistic recovers it —
///    measured mutual information between two *individually rare* exclusive
///    values is genuinely small (with six chains, ruling out one moves another
///    from 1/6 to 1/5), so a purely statistical rule scores them as independent
///    and the funnel marches down the list. Tags are multi-valued and get no
///    such rule.
///
/// 2. **Otherwise, normalized mutual information** between the two presence
///    indicators, which catches co-occurrence a field rule cannot: `lending`
///    and `defi` are different kinds but nearly the same question.
///
/// Measured over a **uniform** distribution, deliberately, not the live
/// posterior. This asks "how related are these two questions in this catalog",
/// which is a property of the data. Under the posterior the answered facet has
/// already collapsed to near-zero mass, its marginal entropy goes with it, and
/// every dependence involving it reads as ~0 — the measure would switch itself
/// off at exactly the moment it is needed.
pub fn facet_dependence(candidates: &[Candidate], a: &FacetRef, b: &FacetRef) -> f64 {
    if candidates.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.kind == b.kind && matches!(a.kind, FacetKind::Category | FacetKind::Chain) {
        return 1.0;
    }

    let w = 1.0 / candidates.len() as f64;
    let mut joint = [0.0f64; 4];
    for candidate in candidates {
        let pa = presence(candidate, a);
        let pb = presence(candidate, b);
        joint[0] += w * pa * pb;
        joint[1] += w * pa * (1.0 - pb);
        joint[2] += w * (1.0 - pa) * pb;
        joint[3] += w * (1.0 - pa) * (1.0 - pb);
    }
    let total: f64 = joint.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    for cell in joint.iter_mut() {
        *cell /= total;
    }

    let p_a = joint[0] + joint[1];
    let p_b = joint[0] + joint[2];
    let marginals = [
        p_a * p_b,
        p_a * (1.0 - p_b),
        (1.0 - p_a) * p_b,
        (1.0 - p_a) * (1.0 - p_b),
    ];

    let mut mi = 0.0;
    for (observed, independent) in joint.iter().zip(&marginals) {
        if *observed > 0.0 && *independent > 0.0 {
            mi += observed * (observed / independent).ln();
        }
    }

    let h_a = entropy(&[p_a, 1.0 - p_a]);
    let h_b = entropy(&[p_b, 1.0 - p_b]);
    let floor = h_a.min(h_b);
    if floor <= 0.0 {
        return 0.0;
    }
    (mi / floor).clamp(0.0, 1.0)
}

/// The next question: `argmax` of information gain **discounted by how much the
/// answers so far already imply it**, skipping facets already answered.
///
/// Plain `argmax` IG walks one dimension. Chains partition the catalog, so each
/// is individually high-gain, and "not Solana?" is reliably followed by "not
/// Ethereum?" — a worse search (the second question is mostly answered by the
/// first) and a worse game. Discounting by `facet_dependence` against every
/// answered facet makes the funnel change subject once a dimension is spent.
///
/// The discount is a scale on gain, never a filter: a facet that is the only
/// informative one left still gets asked, just at a reduced score. So this can
/// reorder questions but cannot strand the funnel with nothing to ask, and
/// `params::DIVERSITY_WEIGHT = 0.0` recovers the previous behaviour exactly.
///
/// `None` when the pool is exhausted or nothing clears
/// `params::MIN_INFORMATION_GAIN` — the caller reads that as "stop asking", not
/// as an error. The floor is applied to raw gain, before the discount, so
/// turning diversity up can never silently stop the funnel early. Ties break
/// toward the earlier pool entry, so question order is deterministic for a given
/// catalog rather than dependent on iteration luck.
pub fn select_question(
    candidates: &[Candidate],
    answers: &[Answer],
    pool: &[FacetRef],
) -> Option<FacetRef> {
    let post = scoring::posterior(candidates, answers);
    let mut best: Option<(&FacetRef, f64)> = None;

    for facet in pool {
        if answers.iter().any(|a| &a.facet == facet) {
            continue;
        }
        let gain = expected_information_gain(candidates, &post, facet);
        if gain < params::MIN_INFORMATION_GAIN {
            continue;
        }
        let implied = answers
            .iter()
            .map(|a| facet_dependence(candidates, facet, &a.facet))
            .fold(0.0f64, f64::max);
        let score = gain * (1.0 - params::DIVERSITY_WEIGHT * implied);
        if best.is_none_or(|(_, top)| score > top) {
            best = Some((facet, score));
        }
    }

    best.map(|(facet, _)| facet.clone())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopReason {
    PosteriorThreshold,
    EntropyFloor,
    QuestionCap,
    NoInformativeQuestion,
}

/// Whether the funnel has learned enough to stop. `None` means keep asking.
///
/// Two confidence rules, per A12: the posterior threshold `p_top1 >= 1 - delta`
/// (Naghshvar & Javidi's retirement rule), and a normalized-entropy floor for
/// the case where the field has genuinely collapsed but no single candidate
/// crosses the threshold.
///
/// A `top1 - top2` margin was **considered and rejected**. Burgener's 20Q patent
/// uses margin for question *selection* only, never for stopping, and no
/// retrieved source shows a margin threshold is principled — it is an unbacked
/// heuristic sitting on the one decision that determines whether the user is
/// shown a confident answer.
///
/// This deliberately does not know about the client's `forceResults` flag; that
/// is the HTTP layer's concern.
pub fn should_stop(
    post: &[f64],
    questions_asked: usize,
    next_question: Option<&FacetRef>,
) -> Option<StopReason> {
    if post.is_empty() || questions_asked >= params::MAX_QUESTIONS {
        return Some(StopReason::QuestionCap);
    }
    if next_question.is_none() {
        return Some(StopReason::NoInformativeQuestion);
    }

    let top = post.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if top >= 1.0 - params::POSTERIOR_STOP_DELTA {
        return Some(StopReason::PosteriorThreshold);
    }

    // Normalizing by `ln n` puts the floor on a 0..1 scale that means the same
    // thing whatever the candidate count; undefined for n == 1, which the
    // posterior threshold has already caught.
    let n = post.len();
    if n > 1 && entropy(post) / (n as f64).ln() <= params::ENTROPY_STOP_FLOOR {
        return Some(StopReason::EntropyFloor);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::{FacetKind, FacetState};

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

    fn chain(value: &str) -> FacetRef {
        FacetRef {
            kind: FacetKind::Chain,
            value: value.into(),
        }
    }

    /// One app per chain plus a tag that cuts across all of them, so the chain
    /// facets are individually high-gain (they partition the field) while the
    /// tag is the only question that asks about something else.
    /// Every app shares one category, so category facets carry no information
    /// and the contest is genuinely between "another chain" and "a tag". The tag
    /// is deliberately sparse, so its raw gain sits BELOW the next chain's — the
    /// funnel only reaches it by trading information gain for a change of
    /// subject, which is exactly the trade `DIVERSITY_WEIGHT` exists to make.
    fn one_category_fixture() -> Vec<Candidate> {
        ["solana", "ethereum", "base", "polygon", "bitcoin", "aptos"]
            .iter()
            .enumerate()
            .flat_map(|(i, ch)| {
                (0..2).map(move |j| {
                    let mut c = candidate(
                        &format!("{ch}{j}"),
                        "defi",
                        if i == 1 && j == 0 { &["custody"] } else { &[] },
                        0.5,
                    );
                    c.chain = (*ch).into();
                    c
                })
            })
            .collect()
    }

    fn multi_chain_fixture() -> Vec<Candidate> {
        ["solana", "ethereum", "base", "polygon", "bitcoin", "aptos"]
            .iter()
            .enumerate()
            .flat_map(|(i, ch)| {
                (0..2).map(move |j| {
                    let mut c = candidate(
                        &format!("{ch}{j}"),
                        if j == 0 { "defi" } else { "nft" },
                        if (i + j) % 2 == 0 { &["custody"] } else { &[] },
                        0.5,
                    );
                    c.chain = (*ch).into();
                    c
                })
            })
            .collect()
    }

    /// The behaviour the whole diversity term exists for: after "no, not
    /// Solana", the funnel must change subject rather than marching down the
    /// chain list. Under plain argmax IG the next pick is another chain, because
    /// chains partition the catalog and so each is individually informative —
    /// but the second chain question is mostly answered by the first.
    #[test]
    fn a_rejected_chain_does_not_lead_to_another_chain() {
        let candidates = one_category_fixture();
        // Raw gain here ranks ethereum (0.335) above custody (0.141): without
        // the diversity discount argmax picks another chain every time.
        let pool = vec![
            chain("solana"),
            chain("ethereum"),
            chain("base"),
            chain("polygon"),
            category("defi"),
            tag("custody"),
        ];
        let answers = vec![Answer {
            facet: chain("solana"),
            value: AnswerValue::No,
        }];

        let next = select_question(&candidates, &answers, &pool).expect("a question remains");
        assert_ne!(
            next.kind,
            FacetKind::Chain,
            "after rejecting a chain the funnel asked about {next:?} — another chain is \
             nearly the same question, since ruling one out just shifts mass onto the rest"
        );
    }

    /// The discount scales gain, it never filters. A facet that is the only
    /// informative one left must still be asked however dependent it is on what
    /// came before, or the funnel would strand itself with questions available.
    #[test]
    fn diversity_reorders_but_never_strands_the_funnel() {
        let candidates = one_category_fixture();
        let answers = vec![Answer {
            facet: chain("solana"),
            value: AnswerValue::No,
        }];
        let only_chains = vec![chain("ethereum"), chain("base")];

        assert!(
            select_question(&candidates, &answers, &only_chains).is_some(),
            "diversity discounted every remaining facet to nothing and stopped the funnel early"
        );
    }

    /// Two values of the same single-valued field are the same question asked
    /// twice, and nothing statistical shows that: with six chains, ruling one out
    /// only moves another from 1/6 to 1/5, so measured mutual information between
    /// them is genuinely small. An overlap measure is worse still — their Present
    /// sets are disjoint, so Jaccard calls them maximally *distant*. Only the
    /// structural rule gets this right.
    #[test]
    fn exclusive_facets_score_as_dependent_not_distant() {
        let candidates = multi_chain_fixture();

        let across_dimensions = facet_dependence(&candidates, &chain("solana"), &tag("custody"));
        let same_dimension = facet_dependence(&candidates, &chain("solana"), &chain("ethereum"));

        assert!(
            same_dimension > across_dimensions,
            "two mutually exclusive chains scored {same_dimension} against {across_dimensions} \
             for a cross-cutting tag — the measure is reading exclusivity as independence"
        );
    }

    /// Ten candidates, all with the same content score so the starting posterior
    /// is uniform and the two facets below differ only in how they discriminate.
    ///
    /// - `tag("wide")`: 5 `Present`, 5 `Unknown` — an exact 50/50 split of the
    ///   posterior mass.
    /// - `category("defi")`: 3 `Present`, 7 `Absent` — a 30/70 split.
    fn split_fixture() -> Vec<Candidate> {
        (0..10)
            .map(|i| {
                let cat = if i < 3 { "defi" } else { "other" };
                let tags: &[&str] = if i % 2 == 0 { &["wide"] } else { &[] };
                candidate(&format!("app{i}"), cat, tags, 0.5)
            })
            .collect()
    }

    #[test]
    fn entropy_matches_the_closed_forms() {
        assert!((entropy(&[0.5, 0.5]) - std::f64::consts::LN_2).abs() < 1e-9);
        assert!(entropy(&[1.0, 0.0]).abs() < 1e-9);
        assert!(entropy(&[]).abs() < 1e-9);
        assert!((entropy(&[0.25; 4]) - 4.0f64.ln()).abs() < 1e-9);
    }

    #[test]
    fn information_gain_is_never_negative() {
        let candidates = split_fixture();
        let post = scoring::posterior(&candidates, &[]);
        let pool = [tag("wide"), category("defi"), category("nft")];

        let gains: Vec<f64> = pool
            .iter()
            .map(|f| expected_information_gain(&candidates, &post, f))
            .collect();
        // This leg cannot fail on its own — `expected_information_gain` floors at
        // zero, so it restates the floor rather than testing the maths. The
        // second assertion below is what stops the test being vacuous, and
        // `information_gain_matches_the_closed_form` is what actually pins the
        // formula.
        for (facet, gain) in pool.iter().zip(&gains) {
            assert!(*gain >= 0.0, "{facet:?} produced negative gain {gain}");
        }
        assert!(
            gains.iter().any(|g| *g > params::MIN_INFORMATION_GAIN),
            "fixture must contain at least one informative facet, got {gains:?}"
        );
    }

    /// Pins the IG arithmetic against a case with a hand-derivable answer, so a
    /// plausible-but-wrong rearrangement cannot pass by staying non-negative.
    ///
    /// Two equally likely candidates, one `Present` and one `Absent`. Both
    /// answers are equally likely (`p = 0.5`), and either one leaves the
    /// posterior at `[1-EPS, EPS]`, so
    /// `IG = ln 2 - H_b(EPS)` exactly, where `H_b` is the binary entropy.
    /// Dropping the branch renormalization — the easy way to get this wrong —
    /// yields 0.247 against the correct 0.495 and fails here.
    #[test]
    fn information_gain_matches_the_closed_form() {
        let candidates = [
            candidate("hit", "defi", &[], 0.5),
            candidate("miss", "nft", &[], 0.5),
        ];
        let post = scoring::posterior(&candidates, &[]);
        assert!((post[0] - 0.5).abs() < 1e-12);

        let e = params::EPS;
        let binary_entropy = -((1.0 - e) * (1.0 - e).ln() + e * e.ln());
        let expected = std::f64::consts::LN_2 - binary_entropy;

        let actual = expected_information_gain(&candidates, &post, &category("defi"));
        assert!(
            (actual - expected).abs() < 1e-12,
            "IG {actual} != closed form {expected}"
        );
    }

    #[test]
    fn a_facet_every_candidate_shares_carries_no_information() {
        let candidates = split_fixture();
        let post = scoring::posterior(&candidates, &[]);

        // Every candidate is on solana, so `Present` for all of them.
        let shared = FacetRef {
            kind: FacetKind::Chain,
            value: "solana".into(),
        };
        assert!(expected_information_gain(&candidates, &post, &shared) < 1e-9);

        // Every candidate is `Unknown` for a tag nobody carries.
        let untagged = tag("nobody-has-this");
        assert!(expected_information_gain(&candidates, &post, &untagged) < 1e-9);
    }

    /// A11: max-IG is **not** the even-split rule. `tag("wide")` splits the
    /// posterior mass exactly 50/50 and is what generalized binary search would
    /// pick; `category("defi")` splits 30/70 but discriminates `Present` against
    /// a true `Absent` rather than against a marginalized `Unknown`, so it
    /// carries more information. Asserting `select_question` picks the 30/70
    /// facet is what stops a future "simplification" back to GBS — a rule
    /// Golovin & Krause show performs very poorly under the soft answers we
    /// deliberately chose.
    #[test]
    fn max_ig_is_not_the_even_split_rule_a11() {
        let candidates = split_fixture();
        let post = scoring::posterior(&candidates, &[]);
        let even = tag("wide");
        let uneven = category("defi");

        // The premise: `even` really is the even-split winner.
        let mass_present = |facet: &FacetRef| -> f64 {
            candidates
                .iter()
                .zip(&post)
                .filter(|(c, _)| c.state(facet) == FacetState::Present)
                .map(|(_, p)| *p)
                .sum()
        };
        assert!((mass_present(&even) - 0.5).abs() < 1e-9);
        assert!((mass_present(&uneven) - 0.3).abs() < 1e-9);

        let ig_even = expected_information_gain(&candidates, &post, &even);
        let ig_uneven = expected_information_gain(&candidates, &post, &uneven);
        assert!(
            ig_uneven > ig_even,
            "IG(30/70) {ig_uneven} must beat IG(50/50) {ig_even}"
        );

        assert_eq!(
            select_question(&candidates, &[], &[even.clone(), uneven.clone()]),
            Some(uneven)
        );
    }

    #[test]
    fn already_answered_facets_are_never_re_asked() {
        let candidates = split_fixture();
        let uneven = category("defi");
        let answers = [Answer {
            facet: uneven.clone(),
            value: AnswerValue::Yes,
        }];

        let picked = select_question(&candidates, &answers, &[uneven.clone(), tag("wide")]);
        assert_eq!(picked, Some(tag("wide")));
        assert_eq!(select_question(&candidates, &answers, &[uneven]), None);
    }

    #[test]
    fn an_empty_pool_yields_no_question() {
        let candidates = split_fixture();
        assert_eq!(select_question(&candidates, &[], &[]), None);
    }

    #[test]
    fn stops_at_the_question_cap() {
        let post = vec![0.25; 4];
        let next = category("defi");
        assert_eq!(
            should_stop(&post, params::MAX_QUESTIONS, Some(&next)),
            Some(StopReason::QuestionCap)
        );
        assert_eq!(
            should_stop(&post, params::MAX_QUESTIONS - 1, Some(&next)),
            None
        );
    }

    #[test]
    fn stops_on_the_posterior_threshold_not_before() {
        let next = category("defi");
        let at = 1.0 - params::POSTERIOR_STOP_DELTA;
        assert_eq!(
            should_stop(&[at, 1.0 - at], 1, Some(&next)),
            Some(StopReason::PosteriorThreshold)
        );

        // Just below the threshold, with normalized entropy comfortably above
        // the floor, the funnel keeps asking.
        let below = at - 0.01;
        let post = [below, 1.0 - below];
        assert!(entropy(&post) / std::f64::consts::LN_2 > params::ENTROPY_STOP_FLOOR);
        assert_eq!(should_stop(&post, 1, Some(&next)), None);
    }

    #[test]
    fn stops_when_no_question_is_informative() {
        assert_eq!(
            should_stop(&[0.25; 4], 1, None),
            Some(StopReason::NoInformativeQuestion)
        );
    }

    /// The funnel must not terminate on turn one with no evidence at all.
    #[test]
    fn a_near_uniform_posterior_does_not_stop() {
        let mut post = vec![1.0 / 20.0; 20];
        post[0] += 0.01;
        post[1] -= 0.01;
        let next = category("defi");
        assert_eq!(should_stop(&post, 0, Some(&next)), None);
    }

    #[test]
    fn stops_on_the_entropy_floor_when_the_field_collapses() {
        // No single candidate crosses the posterior threshold, but the field has
        // genuinely collapsed: one clear leader, one runner-up, a dead tail.
        let mut post = vec![0.0001 / 6.0; 8];
        post[0] = 0.8;
        post[1] = 0.1999;

        let next = category("defi");
        assert!((post.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(post.iter().copied().fold(0.0, f64::max) < 1.0 - params::POSTERIOR_STOP_DELTA);
        assert!(
            entropy(&post) / (post.len() as f64).ln() <= params::ENTROPY_STOP_FLOOR,
            "fixture must sit below the entropy floor for this test to mean anything"
        );
        assert_eq!(
            should_stop(&post, 1, Some(&next)),
            Some(StopReason::EntropyFloor)
        );
    }
}
