# How should a sparse collaborative signal be blended with a content score so it is inert at cold start, grows with support, and is structurally bounded against farming?

## Answer
Use two *separate* mechanisms; one will not do both jobs.

1. **Confidence weighting (cold start, criteria a/b).** Bayesian average / m-estimate: `WR = (v/(v+m))·R + (m/(v+m))·C`. IMDb documents exactly this form. Set `C` = neutral prior (contributes 0), so v=0 ⇒ pure content score. Algolia's documented rule: pick `m` as the lower-quartile of per-item support counts.
2. **A hard cap (criterion c).** Shrinkage does **not** bound an adversary: `v` is attacker-controlled and `v/(v+m) → 1`. Wilson lower bound fails identically (LCB → p̂ as n grows). The cap must be structural and separate: `final = content + λ·learned`, `learned ∈ [0,1]`, `λ < min content gap`.

This cap **is** a named technique: Resnick & Sami's *influence limiter* caps each rater's weight at `β = min(1, R)` and proves (Thm 4) total damage from n sybils is bounded, ≤1/c with λ=log(cn).

Caveat: 2024 poisoning surveys taxonomize defenses as *filtering* + *robust training* — not static caps. Pair the cap with Turnstile/rate-limits.

Hybrid type: Burke's **weighted** hybrid; he warns it assumes component value is "more or less uniform across the space of possible items".

## Citations
- https://help.imdb.com/article/imdb/track-movies-tv/faq-for-imdb-ratings/G67Y87TFYYP6TWAV — IMDb documents `WR = (v/(v+m))R + (m/(v+m))C`, m=25,000, "true Bayesian estimate". Fetched 2026-07-27.
- https://en.wikipedia.org/wiki/Bayesian_average — general form `x̄ = (Cm + Σxᵢ)/(C + n)`; C larger when between-item variation is small. Fetched 2026-07-27.
- https://www.algolia.com/doc/guides/managing-results/must-do/custom-ranking/how-to/bayesian-average — documented implementation; choose C as the 25th-percentile rating count. Fetched 2026-07-27.
- https://presnick.people.si.umich.edu/papers/recsys07/p25-resnick.pdf (via r.jina.ai) — influence limiter; `β_j = min(1, R_j)`; Thm 4 bounds sybil damage; λ=log(cn) ⇒ damage ≤ 1/c. Fetched 2026-07-27.
- https://www.evanmiller.org/how-not-to-sort-by-average-rating.html — Wilson LCB formula; for binary proportions only; corrects small-n, not adversaries. Fetched 2026-07-27.
- https://en.wikipedia.org/wiki/Binomial_proportion_confidence_interval — Wilson (1927), JASA 22(158):209-212; safe for small samples/skew. Fetched 2026-07-27.
- https://arxiv.org/html/2406.01022v1 — defense taxonomy = poisoning-data filtering + robust training; "truncated loss"/"reweighted loss" confidence-aware training; no influence-limiter/trust/reputation mention in retrieved text. Fetched 2026-07-27.
- https://pzs.dstu.dp.ua/DataMining/recom/bibl/Hybrid_Recommender_Systems_Survey_and_Experiments.pdf (via r.jina.ai) — Burke 2002, seven hybrid types; weighted-hybrid uniformity assumption; knowledge-based components have no ramp-up problem. Fetched 2026-07-27.
- https://link.springer.com/article/10.1007/s10462-018-9655-x — shilling-attack review (paywalled, not fetched).

## Confidence: medium
High on the shrinkage formula, Wilson, Burke, and the influence limiter's existence; medium overall because the influence-limiter theorem text came through a text-extraction proxy, not the ACM PDF directly, and because no source I retrieved states the specific claim "a bounded additive term cannot promote a bad match at any N" — that is algebra you must assert and unit-test, not a cited result. Fetching the ACM/Springer originals would raise this.

## Could not verify
- **Exact wording of Resnick & Sami Theorem 4 and the "(n,c)-robustness" definition**: direct PDF fetches (sigecom.org, presnick.people.si.umich.edu) returned binary; the r.jina.ai proxy rendered it and reported the theorem does *not* use the term "(n,c)-robustness". Treat the quoted bound `Σ ΔK_t ≥ −n·e^λ` as proxy-extracted, unconfirmed against the publisher PDF.
- **That the shilling/profile-injection literature recommends a hard cap**: it does not, in anything I retrieved. Springer reviews (s10462-018-9655-x, s10462-012-9364-9) and Mobasher et al. 2007 (TOIT 7:4) are paywalled and were not fetched. Retrieved surveys recommend *detection* (anomaly/clustering filtering) and *robust algorithms*, not capping. **Your promise's framing is defensible as a design invariant but is not the literature's headline defence** — say "bounded-influence blend, plus Turnstile + rate limiting", not "the standard anti-shilling defence".
- **Quantitative evidence that a bounded term limits attack impact in a content+CF hybrid**: not found. Resnick's bound is for a sequential rating aggregator, not an additive two-term ranking score. No paper retrieved measures prediction-shift under a capped additive term.
- **m-estimate provenance (Cestnik 1990)**: appears in search results (dl.acm.org/doi/10.5555/3070070.3070098) but full text not fetched; the link from IMDb's `m` to the ML m-estimate is my inference, not a cited equivalence.
- **Hampel B-robustness / bounded gross-error sensitivity** as the robust-statistics name for your cap: search snippets only, no primary page fetched.
