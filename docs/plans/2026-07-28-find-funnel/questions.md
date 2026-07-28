# Phase 1 questions — all answered

- id: Q1
  question: "Akinator mechanic: (A) guided discovery funnel — user has a need, not an app in mind (B) literal guess-the-app-I'm-thinking-of game (C) both on one engine"
  why: "decides whether this is a recommender or a game; determines whether a dense per-app attribute matrix is required at all"
  answer: "A — guided discovery funnel. Goal restated by user: 'suggest the perfect app for the user'."

- id: Q2
  question: "Question source: (A) data-derived selection + curated phrasing (B) pure data-derived, generic phrasing (C) hand-authored question tree (D) LLM-generated at runtime"
  why: "decides whether the product takes on a paid external dependency, and how much question copy must be hand-maintained as the tag vocabulary grows"
  answer: "A — engine picks WHICH facet by information gain over live data; phrasing from a bounded hand-written map for the 16 fixed categories and the chain list, generic template fallback for crowd-sourced tags. No LLM dependency."

- id: Q3
  question: "Narrowing semantics: (A) soft scoring, never dead-ends (B) hard filters with backtrack (C) hard on fixed enums, soft on tags"
  why: "this is the core engine decision — boolean filtering vs probabilistic scoring — and determines whether a zero-result dead end is even reachable"
  answer: "A — soft scoring. Answers adjust per-app scores; shortlist is always non-empty. Chosen because crowd-sourced tags are sparse, so tag absence is not evidence of absence."

- id: Q4
  question: "Placement: (A) new /find route + 5th nav entry (B) mode toggle on the homepage Discover (C) /find route not in nav"
  why: "decides URL/SEO surface and whether the already-busy homepage route grows a second mode"
  answer: "A — new /find route with its own metadata, plus a nav entry alongside Browse / Rankings / Rewards / About."

- id: Q5
  question: "Engine location: (A) client-side over one fetched catalog snapshot (B) server-side in the indexer (C) hybrid precomputed matrix"
  why: "decides per-question latency, testability, and — as it turned out — whether the paid catalog leaks"
  answer: "B — server-side in the indexer. User's stated reason overrides the latency argument: 'storing the relevant data client side leaks the DB'. The catalog is sold per-request via the x402 Data API, so a client snapshot would give away a monetized asset. This also constrains the endpoint contract, not just the storage location — see A6 in decisions.md."

- id: Q6
  question: "Learning from prior sessions: (A) outcome-weighted results (B) question-quality tuning only (C) both (D) skip learning in v1"
  why: "user's phrasing 'and other user answers' introduced a cross-session learning loop not present in the original framing"
  answer: "C — both. Prior outcomes boost candidate apps for similar answer paths; answer distributions reorder the question pool toward facets that actually discriminate."

- id: Q7
  question: "Training signal: (A) explicit confirm, click as weak secondary (B) click-through only (C) downstream engagement — vote/stake/dwell"
  why: "decides data quality vs volume of the learning loop, and whether a negative signal exists at all"
  answer: "A — 'Is this the one?' Yes is the strong signal; click-through trains at low weight; 'Not quite' is a negative signal."

- id: Q8
  question: "Anti-gaming: (A) structural cap on the learned term + Turnstile/rate-limit (B) per-identity contribution caps (C) require SIWS wallet (D) ship open, harden later"
  why: "surfacing in /find drives traffic → ad revenue → staker payout, so farming confirmations is directly profitable; PRODUCT.md names 'hard to game' as a success criterion"
  answer: "A — learned term capped at a fixed fraction of the final score so farming cannot lift a bad match at any volume; reuse existing Turnstile + fixed-window rate limiter on the confirm endpoint."

- id: Q9
  question: "Termination: (A) auto-stop on separation + hard cap + always-available escape (B) fixed length (C) run until user stops"
  why: "quiz length is a core feel decision, and interacts with the self-tuning question loop"
  answer: "A — stop when the top candidate separates by a threshold, hard cap 8 questions, 'show me results' / back / start-over available from question one."
