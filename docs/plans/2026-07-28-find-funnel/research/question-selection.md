# How do soft-scored 20-questions engines pick the next question, and is "closest to 50/50" really max information gain?

## Answer

**Criterion: expected information gain = mutual information over the normalized candidate weight distribution.** With belief `p(y_i | X_{t-1})` (your scores, normalized to sum 1) and per-candidate answer likelihoods `p(r_k | q, y_i)`:

- `p(r_k | X_{t-1}, q) = Σ_i p(y_i | X_{t-1}) · p(r_k | q, y_i)`
- `H(y | X_{t-1}, q) = Σ_{r_k ∈ R(q)} p(r_k | X_{t-1}, q) · H(y | X_{t-1}, q, r_k)`
- `IG(y; q | X_{t-1}) = H(y | X_{t-1}) − H(y | X_{t-1}, q)` — pick argmax.

Multi-valued answers (yes/no/don't-care/probably) are just the values of `R(q)`; softness lives in `p(r_k|q,y_i)`, not in filtering. Not Gini — Gini is the CART impurity variant, not what this literature uses.

**The 50/50 heuristic is exact only for deterministic answers.** `I(C;A) = H(A) − H(A|C)`; if the answer is a deterministic function of the candidate, `H(A|C)=0`, so `IG = H(A)`, maximized by a uniform (50/50 binary) weighted split. Once answers are soft/noisy, `H(A|C) > 0` and the two diverge — Golovin & Krause: generalized binary search (the even-split rule) "can perform very poorly" under noise. **So: an approximation, not an equivalence.** Burgener's 20Q patent uses precisely the weighted-totals even-split rule, confirming it as the industry heuristic rather than the correct objective.

**Greedy is defensible:** Jedynak/Frazier/Sznitman prove any policy maximizing one-step expected entropy reduction "is also optimal over the full horizon" — for their question class (arbitrary subsets), so treat it as support, not proof, for a fixed facet list.

**Stopping:** cite a posterior threshold, not a magic margin. Naghshvar & Javidi retire when `ρ_i ≥ 1 − L⁻¹` (L = penalty for a wrong declaration). Top1−top2 margin has a documented precedent (Burgener "greatest margin" confirmation mode) but no optimality proof found. Yu et al. instead learn STOP/ASK from top-k probabilities and turn number.

**Cost:** `O(N · Σ_f |R(f)|)` per question. 135 × 300 facets × 4 answers ≈ 1.6e5 ops — negligible in Rust. 10k candidates ≈ 1.2e7 ops/question, still single-digit ms. Arithmetic from the formula, not a benchmark.

## Citations

- https://arxiv.org/abs/1911.03598 (ar5iv HTML, fetched 2026-07-27) — Yu, Chen, Wang, Lei, Artzi, ACL 2020. Exact EIG formulas over a soft belief distribution with multi-choice answers; belief update `p(y_i|X_t) ∝ p(y_i|x) Π_τ p(r_τ|q_τ,y_i)`; learned STOP/ASK controller on top-k probabilities.
- https://en.wikipedia.org/wiki/Mutual_information (fetched 2026-07-27) — `I(X;Y) = H(X)−H(X|Y) = H(Y)−H(Y|X)`; deterministic Y ⇒ `H(Y|X)=0` ⇒ `I = H(Y)`. This is the whole 50/50-equivalence argument.
- https://arxiv.org/abs/1010.3091 (fetched 2026-07-27) — Golovin, Krause, Ray, "Near-Optimal Bayesian Active Learning with Noisy Observations": GBS = "select a query that most evenly splits the hypotheses"; under noise "GBS can perform very poorly". Establishes the heuristic is not the objective.
- https://arxiv.org/abs/0910.4397 (fetched 2026-07-27) — Nowak, defines GBS as the most-even-split greedy rule.
- https://www.cambridge.org/core/journals/journal-of-applied-probability/article/.../6F51F7E5CE7D0B1ED4CEF0221D0B6E08 (fetched 2026-07-27) — Jedynak, Frazier, Sznitman, J. Appl. Prob. 49(1):114–136, 2012: "any policy optimizing the one-step expected reduction in entropy is also optimal over the full horizon."
- https://arxiv.org/pdf/1203.4626 (ar5iv HTML, fetched 2026-07-27) — Naghshvar & Javidi, *Active Sequential Hypothesis Testing*, Ann. Statist. 41(6), 2013. Stopping rule: "If ρi≥1−L−1, retire and select Hi as the true hypothesis." Two-phase policy switches to top-1-vs-rest discrimination once `ρ_i ≥ ρ̃ > 0.5`.
- https://patents.google.com/patent/US20060230008A1/en (fetched 2026-07-27) — Burgener, "Artificial neural network guessing method and game" (20Q). Documented soft-scored engine: weights ±1..±4, "Unknown is not counted as an answer"; selection method 1 = "the question with the lowest total difference"; method 2 = "the question with the greatest margin". No entropy anywhere.
- https://en.wikipedia.org/wiki/Akinator (fetched 2026-07-27) — "Implementation details are not shared"; Limule by Elokence, "runs on an internally designed algorithm".

## Confidence: high

Every load-bearing claim carries a fetched primary source; the 50/50-vs-IG result is arithmetic from a cited identity. Would drop to medium if your answer model turns out non-conditionally-independent across questions (the ACL belief update assumes a product of likelihoods) — that assumption is yours to check, not theirs.

## Could not verify

- **Akinator's actual criterion.** Searched for Limule/Elokence algorithm descriptions and patents; the official site returned HTTP 403 and Wikipedia states details are not shared. Only low-quality secondary sources (Medium, Quora, Grokipedia) assert "ID3/information gain" — none authoritative. Everything above is open-literature substitute, labelled as such.
- **A proof that top1−top2 margin is a principled stopping rule.** Found only the Burgener patent's "greatest margin" *question-selection* mode and Naghshvar–Javidi's *posterior-threshold* retirement. No source retrieved proves a margin threshold optimal. Recommend the posterior threshold (`p_top1 ≥ 1−δ`, or a normalized-entropy floor) as the citable rule; margin is an unbacked heuristic.
- **Burgener patent: whether the two totals are strictly weighted sums and over how many top objects.** The fetch's answer was hedged ("does not explicitly specify a fixed N"); I did not read the raw claim text line-by-line. Treat the quoted phrases as accurate but the weighted-vs-count detail as ~medium.
- **Golovin/Krause and Jedynak PDFs.** Direct PDF fetches failed to decode (binary); relied on the arXiv abstract page and Cambridge Core abstract respectively. The EC² algorithm's internals are therefore uncited — only its critique of GBS is.
- **Whether the ASAPP reference implementation actually codes the EIG formula.** Fetched https://github.com/asappresearch/interactive-classification; README is one line and I did not open source files.
- **Any benchmark for the cost claim.** Pure operation-count arithmetic; no measured Rust timing retrieved or run.
