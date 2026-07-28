# What is the standard citable update rule for soft-scoring candidates from noisy yes/no/unknown answers over sparse, incomplete boolean attributes?

## Answer
Accumulate naive-Bayes log-likelihoods with a bounded noise floor and a **three-valued** attribute state.

`score(c) = log P(c) + Σ_q log L(a_q | c)`.

Per question on attribute X:
- **known present** → `L = P(a|X=1)`; **known absent** → `L = P(a|X=0)`.
- **never recorded** → marginalise the latent value: `L = π_X·P(a|X=1) + (1−π_X)·P(a|X=0)`, π_X = tag prevalence. This is the standard missing-value treatment; with π chosen so L is answer-independent it degenerates to *omitting the term entirely*, which is what documented NB implementations do.
- **"don't care"** → no update (factor 1), same mechanism.

Untagged ≠ absent is the PU-learning/SCAR point: unlabeled items are a mixture, not negatives.

Noise: clamp `P(a|X) ∈ [ε, 1−ε]`, ε>0 (the PBA `p_c`/`q_c=1−p_c` form). No likelihood is ever 0, so no candidate is eliminated and one wrong answer costs a bounded `log((1−ε)/ε)` that later answers outvote.

Smoothing: additive α on tag counts (α=1 Laplace, α=0.5 Jeffreys) = symmetric Dirichlet prior; prevents zeros.

Log-space is the normal choice, to avoid float underflow when multiplying many small probabilities.

## Citations
- https://nlp.stanford.edu/IR-book/html/htmledition/naive-bayes-text-classification-1.html — Manning IR §13.2: a single zero probability nullifies all other evidence (eq. 118); add-one/Laplace smoothing (eq. 119) as "a uniform prior"; "better to perform the computation by adding logarithms of probabilities instead of multiplying" to prevent floating-point underflow (eq. 115).
- https://search.r-project.org/CRAN/refmans/e1071/html/naiveBayes.html — documented OSS implementation: "For attributes with missing values, the corresponding table entries are omitted for prediction."
- https://patents.google.com/patent/US20060230008A1/en — Burgener 20Q patent, the documented 20-questions engine: graded answer weights (Yes +4, Probably +3, Sometimes +2, Maybe +1, Unknown 0, Doubtful −1, Rarely −2, No −4) added/subtracted per cell polarity; "The answer 'Unknown' is not counted as an answer and is not used in these calculations." Soft additive scoring, no elimination.
- https://ar5iv.labs.arxiv.org/html/1711.00843 — Generalized Probabilistic Bisection (Horstein lineage), Lemma 2.1: `g_{n+1}(u) = 1/γ_n · [p(x_n)·1{u≥x} + (1−p(x))·1{u<x}]·g_n(u)`; oracle "produces a correct sign only with probability p(x_n)", p(x) ≥ 0.5. This is the noise-parameter update; because 1−p > 0 the posterior mass is scaled, never zeroed.
- https://ar5iv.labs.arxiv.org/html/2404.00145 — SCAR in positive-unlabeled learning: propensity `e(x)=c`; "unlabeled observations can belong either to the positive class or to the negative class". Justifies never treating a missing tag as negative evidence.
- https://scikit-learn.org/stable/modules/naive_bayes.html — "The smoothing priors α ≥ 0 account for features not present in the learning samples and prevent zero probabilities… Setting α = 1 is called Laplace smoothing, while α < 1 is called Lidstone smoothing."
- https://en.wikipedia.org/wiki/Additive_smoothing — "Common choices for α are 0 (no smoothing), +1⁄2 (the Jeffreys prior), or 1 (Laplace's rule of succession)"; equals the posterior mean under a symmetric Dirichlet prior. (Encyclopedic, cites Jurafsky & Martin 2008 and Russell & Norvig 2010; I did not fetch those.)
- https://github.com/rogeriochaves/bayes-akinator (`server.py`) — hobby OSS Akinator: unknown attribute defaults to 0.5 (neutral, no push either way); `P *= 1 - abs(answer - char_answer)`; multiplies in probability space, i.e. does *not* use logs, so it carries the underflow risk Manning warns about.
- Jedynak, Frazier & Sznitman, "Twenty Questions with Noise: Bayes Optimal Policies for Entropy Loss", J. Appl. Prob. 49(1):114–136, 2012 — https://www.cambridge.org/core/journals/journal-of-applied-probability/article/twenty-questions-with-noise-bayes-optimal-policies-for-entropy-loss/6F51F7E5CE7D0B1ED4CEF0221D0B6E08 — noisy 20-questions as sequential Bayesian posterior update; greedy one-step entropy reduction is optimal over the full horizon (supports greedy max-information question selection). Abstract-level only; see Could not verify.
- Pelc, "Searching games with errors — fifty years of coping with liars", TCS 270:71–109, 2002 — https://www.sciencedirect.com/science/article/pii/S0304397501003036 — survey framing of search where some answers are erroneous. Abstract-level only.

## Confidence: medium
High on the individual mechanisms (marginalise/omit missing, ε-bounded likelihoods, additive smoothing, log-space) — each is directly quoted from a fetched source. Medium overall because no single fetched source presents the *combined* recipe for sparse crowd-tags, and because every PDF fetch failed in this environment (no poppler), so Jedynak et al., Elkan & Noto, and Saar-Tsechansky & Provost are cited at abstract/search-snippet level only. Fetching those three full texts would raise it.

## Could not verify
- **Jedynak/Frazier/Sznitman exact update equations and the claim that no candidate is eliminated**: fetched https://people.orie.cornell.edu/pfrazier/pub/2011_JedynakFrazierSznitman_20questions.pdf twice; PDF text extraction unavailable in this environment. Only the abstract was read. The ε-bounded-likelihood claim is instead sourced from the PBA paper (ar5iv HTML), which does state it.
- **Elkan & Noto 2008 `p(y=1|x) = p(s=1|x)/c`**: fetched https://cseweb.ucsd.edu/~elkan/posonly.pdf — unreadable PDF. SCAR is cited from arXiv 2404.00145 instead, which references Elkan & Noto but does not state that formula. Do not quote the formula as verified.
- **A citable justification for choosing a *specific* smoothing constant for this use case**: found only the generic α∈{0.5, 1} conventions. I did not find a retrievable source recommending a smoothing constant for sparse crowd-sourced tag matrices. Searched: additive smoothing constant selection, Zhai & Lafferty Dirichlet μ tuning (not fetched). Treat α as a tunable, validated on held-out sessions.
- **A citable standard for a *damped/partial* update on "don't care"**: no source found endorsing partial updates. Both documented implementations found (e1071, 20Q patent) do a full no-op. The `bayes-akinator` 0.5-default is a neutral no-op in effect, not a damped update.
- **"Probably" / graded answers as likelihoods**: the only retrieved treatment is the 20Q patent's additive integer weights, which is not a probabilistic likelihood. I found no retrievable source giving a principled likelihood for hedged answers; interpolating `L = λ·P(a=yes|X) + (1−λ)·P(a=no|X)` is a plausible but UNVERIFIED extension.
- **Akinator's actual algorithm**: proprietary/closed-source; searches returned only blog restatements. Not usable as a citation.
- **Whether "one wrong answer" recovery is standardly handled by anything beyond a bounded ε** (e.g. explicit answer-revision, Rényi–Ulam lie budgets): Pelc's survey is the right pointer but was fetched at abstract level only, so no concrete mechanism is verified.
