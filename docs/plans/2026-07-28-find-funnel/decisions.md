# Decision ledger

Every choice made but not explicitly stated by the user. `status: assumed`
entries block completion — buildteam:challenging-assumptions resolves them.

- id: A1
  phase: 1
  node: null
  route: must-ask
  claim: "In-flight funnel state is stateless — the client posts its answer history with each request and the server returns the next question plus the current shortlist. Only completed sessions are persisted."
  because: "smallest thing that works with a server-side engine; avoids a session table with TTL/GC for state that lives for ~30 seconds. User specified where the engine runs, never how in-flight state is held."
  affects: [brief.md, API shape, indexer schema]
  closes: []
  status: overridden

- id: A2
  phase: 1
  node: null
  route: must-ask
  claim: "Completed sessions are logged against the existing anonymous visitorId/sessionId tracking identity (as PageView already does), with no PII and no wallet requirement."
  because: "follows the precedent already set by app/src/lib/tracking.ts and the PageView table; keeps /find usable with no wallet per Principle 3. But answer-path logging is a new data category the existing privacy stance was not written for."
  affects: [indexer migration 009_, privacy posture]
  closes: []
  status: confirmed

- id: A3
  phase: 1
  node: null
  route: judgment
  claim: "The result shortlist renders with the existing AppCard component rather than a bespoke result card."
  because: "PRODUCT.md Principle 1 requires vote/stake to live on the card; AppCard already satisfies it and is the established pattern."
  affects: [UI]
  closes: []
  status: verified

- id: A4
  phase: 1
  node: null
  route: judgment
  claim: "/find is entirely read-only and requires no wallet connection; it works identically in simulation and on-chain mode."
  because: "PRODUCT.md Principle 3; the funnel reads catalog data and writes only anonymous session outcomes, neither of which touches Solana."
  affects: [brief.md success criteria]
  closes: []
  status: verified

- id: A5
  phase: 1
  node: null
  route: judgment
  claim: "The bare /find page is indexed; in-progress/parameterized states are noindex-follow with a canonical to /find."
  because: "mirrors the existing homepage canonical/noindex logic in app/src/app/page.tsx, which exists precisely to stop near-duplicate parameterized states diluting ranking signal."
  affects: [SEO, app/src/app/find/page.tsx]
  closes: []
  status: verified

- id: A6
  phase: 1
  node: null
  route: must-ask
  claim: "The /find API contract returns only the next question and a bounded shortlist (suggest ~5 apps max, already-public fields only). It never returns the remaining candidate set, per-app facet vectors, or scores for non-shortlisted apps."
  because: "direct consequence of the user's leak constraint — a server-side engine that echoes the candidate set with facets leaks the catalog exactly as a client snapshot would, one HTTP call deeper. The specific bound (5) is mine, not the user's."
  affects: [API shape, brief.md success criterion 3]
  closes: []
  status: verified

- id: A7
  phase: 1
  node: null
  route: judgment
  claim: "The 8-question cap, the ~0.25 learned-term weight, the 0.2 click-through weight, and the separation threshold are tunable starting values, not fixed requirements. They live in one named-constants module, not scattered as literals."
  because: "none of these can be chosen correctly before there is real session data; the build must make them cheap to change rather than pretend they are known."
  affects: [engine implementation]
  closes: []
  status: verified

- id: A8
  phase: 1
  node: null
  route: judgment
  claim: "New tables land in a single new indexer migration (next number: 009_), and app/ gains no database client."
  because: "AGENTS.md states this as a hard rule — indexer/migrations/ is the one source of DDL for the whole product."
  affects: [indexer/migrations/]
  closes: []
  status: verified

- id: A9
  phase: 1
  node: null
  route: must-ask
  claim: "'Not quite' records a negative signal against the shown top candidate for that answer path and lets the user keep going (next-best suggestion or more questions), rather than ending the session."
  because: "user approved 'Not quite' as a negative signal but never said what it does to the session. Ending on a miss wastes the most informative moment in the funnel."
  affects: [UI flow, training signal]
  closes: []
  status: confirmed

- id: A11
  phase: 2
  node: null
  route: must-ask
  claim: "Question selection uses expected information gain (mutual information) over the normalized candidate weight distribution — NOT the 'splits the set closest to 50/50' rule the user was shown in Q2's preview."
  because: "research/question-selection.md (confidence HIGH) establishes the even-split rule is exactly equivalent to max IG only when answers are deterministic given the candidate. Our answers are explicitly soft/noisy, and Golovin & Krause show generalized binary search 'can perform very poorly' under noise. The user approved a mechanic described in terms the research contradicts; behaviour is the same in spirit (ask the most discriminating question) but the implementation differs."
  affects: [brief.md, questions.md Q2, engine implementation]
  closes: []
  status: verified

- id: A12
  phase: 2
  node: null
  route: must-ask
  claim: "The funnel stops on a posterior-probability threshold (p_top1 >= 1 - delta) or a normalized-entropy floor — NOT the top1-minus-top2 'separation margin' the user was shown in Q9's preview."
  because: "research/question-selection.md: no retrieved source proves a margin threshold optimal; Naghshvar & Javidi's posterior-threshold retirement rule is the citable one. Burgener's patent uses margin only for question selection, never for stopping. The product behaviour the user approved ('stop when confident, cap at 8, always escapable') is preserved exactly."
  affects: [brief.md, questions.md Q9, engine implementation]
  closes: []
  status: verified

- id: A13
  phase: 2
  node: null
  route: must-ask
  claim: "The scoring model assumes answers are conditionally independent given the candidate app (a product of per-answer likelihoods). This assumption is FALSE for our facet set — an app tagged 'lending' is almost certainly category 'defi' — so correlated evidence will be double-counted and over-sharpen the posterior."
  because: "research/question-selection.md's own confidence line flags this as the assumption that would drop it to medium, and explicitly says it is ours to check, not theirs. Naive Bayes tolerates this in classification because argmax is robust to miscalibration, but we use the posterior for a STOPPING threshold, where over-sharpening makes the funnel stop early and over-confident."
  affects: [engine implementation, stopping rule, A12]
  closes: []
  status: verified

- id: A14
  phase: 2
  node: null
  route: verifiable
  claim: "Attributes are three-valued — present / known-absent / never-recorded — with likelihoods accumulated in log space, clamped to [eps, 1-eps] so no candidate is ever eliminated, and 'don't care' performing no update at all."
  because: "research/soft-scoring-update.md, confidence MEDIUM. The never-recorded case marginalises over a tag-prevalence prior rather than counting as negative evidence, which is what stops well-tagged apps dominating thinly-tagged ones. 'Don't care' as a strict no-op is the one point where both documented implementations (R e1071, Burgener 20Q patent) agree."
  affects: [engine implementation]
  closes: []
  status: verified

- id: A15
  phase: 2
  node: null
  route: judgment
  claim: "v1 answer vocabulary is exactly Yes / No / Don't care. Hedged answers ('probably', 'probably not') are out of scope."
  because: "research/soft-scoring-update.md could not find any citable likelihood for hedged answers — the only precedent is the 20Q patent's additive integer weights, which are not probabilities. Shipping an unsourced likelihood into the one term that feeds the stopping rule is worse than not offering the answer."
  affects: [UI, engine implementation]
  closes: []
  status: verified

- id: A16
  phase: 2
  node: null
  route: must-ask
  claim: "Cold start and anti-farming need TWO separate mechanisms, not one: confidence shrinkage ((v/(v+m))R + (m/(v+m))C) inside the learned term for cold start, plus a separate hard cap lambda < (minimum content-score gap) on the term's contribution. The 'farming cannot lift a bad match at any N' property is algebra we assert and unit-test, not a cited result."
  because: "research/cold-start-blending.md, confidence MEDIUM: shrinkage does NOT bound an adversary because v is attacker-controlled and v/(v+m) -> 1; the Wilson lower bound fails identically. The cap has a real named ancestor (Resnick & Sami's influence limiter, RecSys 2007) but no retrieved source measures prediction-shift under a capped additive term in a content+CF hybrid."
  affects: [brief.md success criterion 4, engine implementation]
  closes: []
  status: verified

- id: A17
  phase: 2
  node: null
  route: judgment
  claim: "The tag-prevalence prior (pi_X), the smoothing constant (alpha), the shrinkage constant (m), and lambda have no citable correct values and are tuned on held-out sessions. They live beside the other tunables from A7."
  because: "both research files explicitly say the literature does not pin these down for this setting; Algolia's documented heuristic (m = lower quartile of per-item support) is the only concrete rule retrieved, and it presumes support data we do not yet have."
  affects: [engine implementation]
  closes: []
  status: verified

- id: A18
  phase: 2
  node: null
  route: judgment
  claim: "Anti-gaming is described in the codebase and any user-facing copy as a 'bounded-influence blend, plus Turnstile and rate limiting' — never as 'the standard anti-shilling defence'."
  because: "research/cold-start-blending.md explicitly corrects this framing: retrieved 2024 poisoning surveys taxonomise defences as data filtering plus robust training, and recommend detection, not static caps. The cap is defensible as a design invariant; overstating its pedigree in comments would mislead the next reader."
  affects: [documentation, code comments]
  closes: []
  status: verified

- id: A19
  phase: 3
  node: 1
  route: must-ask
  claim: "The wire contract is two routes: POST /find/next {answers[]} -> {question, shortlist, candidateCount, questionsAsked, done}, and POST /find/confirm {answers, appId, outcome, visitorId, sessionId} -> {ok:true}, idempotent per (sessionId, appId, outcome)."
  because: "no spec existed; shapes follow the existing handlers/ convention (POST /apps/search, POST /track) and the {\"error\": msg} error body every indexer route already returns. Stateless per A1 — the full answer history rides on every request."
  affects: [1.A, 1.F, 1.G, 1.H]
  closes: []
  status: verified

- id: A20
  phase: 3
  node: 1
  route: must-ask
  claim: "`shortlist` is empty unless `done` is true; mid-funnel progress is conveyed by `candidateCount` (an integer) alone. Shortlist is capped at 5 entries of AppDTO."
  because: "direct consequence of the user's leak constraint (A6). Returning ranked apps every turn would let a caller sweep answer combinations and enumerate the catalog one HTTP call at a time — the same disclosure a client-side snapshot would make, just slower. AppDTO itself is already public via /apps/search, so the shortlist at the end is not a new disclosure. The specific cap of 5 is mine."
  affects: [1.F, 1.G, 1.H]
  closes: []
  status: verified

- id: A21
  phase: 3
  node: 1
  route: judgment
  claim: "Facet vocabulary is FacetRef{kind: category|chain|tag, value: slug}; answers are yes|no|skip; internal facet state is three-valued Present|Absent|Unknown. Category and chain are total (every App row has exactly one, so non-matching values are true Absent); tags are sparse (missing tag is Unknown, never Absent)."
  because: "the total-vs-sparse distinction is the mechanism that stops well-tagged apps dominating thin ones (A14). It is a property of the actual schema — App.category and App.chain are NOT NULL with defaults, AppTag rows are opt-in — not a modelling preference."
  affects: [1.B, 1.C, 1.D]
  closes: []
  status: verified

- id: A22
  phase: 3
  node: 1
  route: judgment
  claim: "Rust layout is indexer/src/find/{mod,facets,scoring,selection,blend,params}.rs (pure: scoring, selection, blend) plus indexer/src/handlers/find.rs exposing `pub fn routes() -> Router<Arc<ApiState>>`. Four wiring registrations: `mod find;` in main.rs, `pub mod find;` in handlers/mod.rs, `.merge(crate::handlers::find::routes())` in api.rs, and the NAV entry in Navbar.tsx."
  because: "mirrors the existing handlers/ module convention exactly (every sibling exposes the same routes() shape and is merged in api.rs lines ~121-132); the pure/IO split mirrors ranking.ts and reward_math.rs as AGENTS.md requires."
  affects: [1.C, 1.D, 1.E, 1.F, 1.I]
  closes: []
  status: verified

- id: A23
  phase: 3
  node: 1
  route: judgment
  claim: "app/src/lib/indexerClient.ts gains exactly two exports: fetchNextFindQuestion(input): Promise<FindNextResult> and recordFindOutcome(input): Promise<{ok: true}>. Both app routes wrap handler() + ok()/fail() + Zod schemas findNextSchema/findConfirmSchema from @/lib/validation."
  because: "root AGENTS.md states this route convention as a hard rule; the two-function surface keeps the app's knowledge of the funnel to 'ask for a question' and 'report an outcome', with no scoring logic crossing into TypeScript."
  affects: [1.G, 1.H]
  closes: []
  status: verified

- id: A24
  phase: 3
  node: 1
  route: judgment
  claim: "The build partitions into nine components A-I (migration/persistence, facets, scoring, selection, blend, indexer HTTP seam, app API seam, UI, copy+nav), each with its own runnable acceptance command."
  because: "the three pure-maths components (C/D/E) are separable because their contracts are plain structs, and they carry the highest defect risk, so isolating them makes each independently testable without a database. F and I are seam nodes writable from the pinned wiring table alone."
  affects: [tree/]
  closes: []
  status: verified

- id: A10
  phase: 1
  node: null
  route: judgment
  claim: "Question phrasing for the 16 fixed categories and the chain list is authored as a constants map; crowd-sourced tags use a generic template. Facet vocabulary is defined once and shared, not duplicated between question selection and scoring."
  because: "follows directly from Q2's answer; the shared-vocabulary point is mine, and matters because the engine is in Rust while CATEGORIES/CHAINS are currently TS constants in app/src/lib/constants.ts — that duplication needs an explicit owner."
  affects: [indexer, app/src/lib/constants.ts]
  closes: []
  status: verified

- id: A25
  phase: 4
  node: 1
  route: judgment
  claim: "Node 1 splits into nine children: 1.1 rust foundation seam (find/mod.rs, find/params.rs, main.rs), 1.2 facets, 1.3 scoring+selection, 1.4 blend, 1.5 migration+store, 1.6 indexer HTTP seam (handlers/find.rs, handlers/mod.rs, api.rs), 1.7 app seam (types/validation/constants/indexerClient/Navbar), 1.8 app API routes, 1.9 UI. Scoring and selection are ONE child, not two."
  because: "matches the A24 partition A-I with two adjustments: `find/mod.rs` + `params.rs` are lifted into a foundation seam because every sibling imports them and a file's top-level declarations are seam territory; and C+D are merged because selection.rs must call scoring.rs's `answer_likelihood`, so splitting them would leave 1.4's tests uncompilable until 1.3 landed."
  affects: [tree/1.1..1.9]
  closes: []
  status: verified

- id: A26
  phase: 4
  node: 1
  route: verifiable
  claim: "The acceptance check `curl -s localhost:3000/find | grep -q 'find-funnel'` is replaced by `npm run build --prefix app -> exit 0` plus a grep for the `find-funnel` test id in source."
  because: "no Postgres, no indexer process and no dev server may be started in this environment, so the curl check cannot run. `next build` compiles and type-checks the route for real and was verified to exit 0 at baseline, which is the strongest command that DOES run."
  affects: [1, 1.9]
  check: "cd app && npm run build && grep -rn 'find-funnel' src/app/find src/components/find"
  closes: []
  status: verified

- id: A27
  phase: 4
  node: 1
  route: verifiable
  claim: "The A13 conditional-independence mitigation is a likelihood-exponent damping factor `CORRELATION_DAMPING = 0.7 < 1` applied to every accumulated log-likelihood term, named in params.rs with a doc comment, not a silent TODO."
  because: "A13 requires an explicit mitigation from whichever node owns scoring/selection. Of the three candidates (exponent damping, excluding correlated facets, a calibrated threshold), damping is the one that needs no correlation estimates from session data we do not have yet, and it degrades gracefully: it only slows the posterior's sharpening, so the A12 stopping threshold fires later rather than early-and-overconfident."
  affects: [1.1, 1.3]
  check: "grep -n CORRELATION_DAMPING indexer/src/find/params.rs indexer/src/find/scoring.rs"
  closes: []
  status: verified

- id: A28
  phase: 4
  node: 1
  route: judgment
  claim: "`visitorId`/`sessionId` for /find/confirm are derived server-side by `resolveVisitor(req.headers)` in the Next.js route, never accepted from the request body. The indexer's wire contract still carries them as fields, populated by that route."
  because: "mirrors app/src/app/api/track/route.ts exactly. A client-supplied visitor identity would make the (sessionId, appId, outcome) idempotency key attacker-chosen, i.e. free confirmation farming — which is the specific thing brief success criterion 4 exists to prevent. Supersedes the literal reading of tree/1.md's confirm request example."
  affects: [1.7, 1.8]
  closes: []
  status: verified

- id: A29
  phase: 4
  node: 1
  route: must-ask
  claim: "/api/find/confirm rejects with 403 when Turnstile is CONFIGURED and the token fails, and records the outcome normally when TURNSTILE_SECRET_KEY is unset."
  because: "verifyTurnstileToken returns false both for a failed token and for an unconfigured environment, so hard-gating unconditionally would break /find entirely in local/simulation mode — which violates the requirement that the funnel work identically with no wallet and no infrastructure (A4). Checking the env var explicitly is the only way to tell the two cases apart."
  affects: [1.8]
  closes: []
  status: overridden

- id: A30
  phase: 4
  node: 1
  route: judgment
  claim: "/api/find/next reuses RATE_LIMITS.read (60/min) and /api/find/confirm reuses RATE_LIMITS.auth (10/min) rather than adding a new `find` bucket."
  because: "app/src/lib/api.ts is outside node 1's scope, so a new RATE_LIMITS entry cannot be added without widening the partition. `auth`'s tight 10/min window is the right shape for the write path anyway — it exists precisely to blunt repeated cheap-to-forge submissions."
  affects: [1.8]
  check: "grep -n 'RATE_LIMITS' app/src/app/api/find/next/route.ts app/src/app/api/find/confirm/route.ts"
  closes: []
  status: verified

- id: A31
  phase: 4
  node: 1
  route: judgment
  claim: "Migration 009 creates two tables: \"FindSession\" (id, visitorId, sessionId, appId, outcome, answers JSONB, questionsAsked, createdAt) with a UNIQUE index on (sessionId, appId, outcome) giving A19's idempotency for free, and \"FindFacetStat\" (facetKind, facetValue, yesCount, noCount, skipCount, updatedAt) keyed on (facetKind, facetValue) for the question-ordering half of the learning loop."
  because: "A1 makes in-flight state stateless, so the only thing that needs persisting is a completed session. Storing the answer path as JSONB rather than a child table keeps the write to one statement and the path is only ever read whole. FindSession also makes brief success criterion 7 (average questions-to-confirm) a single query."
  affects: [1.5, 1.6]
  check: "grep -n 'FindSession\\|FindFacetStat' indexer/migrations/009_find_funnel.sql"
  closes: []
  status: verified

- id: A32
  phase: 4
  node: 1
  route: judgment
  claim: "`Candidate.content_score` is the app's rankScore min-max normalized into [0,1] across the candidate set, and the `content` term of the blend is the normalized POSTERIOR after answers — not the raw quality score. `MIN_CONTENT_GAP` is documented as the posterior gap a genuinely good match is expected to open over a genuinely bad one, and LAMBDA must stay strictly below it."
  because: "the blend has to bound the learned term against the thing the funnel actually ranks on, which is the answer-conditioned posterior. Posteriors are continuous, so there is no literal minimum gap in the data; the honest invariant is the algebra `content_a - content_b > LAMBDA => order preserved for any support`, which is exactly what blend.rs unit-tests (brief success criterion 4)."
  affects: [1.2, 1.3, 1.4, 1.6]
  closes: []
  status: verified

- id: A33
  phase: 4
  node: 1
  route: judgment
  claim: "Expected information gain is computed over the answer set {Yes, No} only. `Skip` is excluded from the IG sum."
  because: "Skip performs no scoring update at all (A15, and both documented precedents agree), so its conditional entropy equals the current entropy and it contributes exactly zero information by construction. Including it in the normalization would shrink every facet's IG by the same unknown factor p(skip), which the engine has no data to estimate."
  affects: [1.3]
  closes: []
  status: verified

- id: A34
  phase: 4
  node: 1
  route: judgment
  claim: "The facet pool is the 16 fixed category slugs, the 8 fixed chain slugs, and every tag slug carried by at least MIN_TAG_SUPPORT (3) candidates. Category and chain slug lists are duplicated in Rust in indexer/src/find/facets.rs."
  because: "A10 flagged the CATEGORIES/CHAINS duplication as needing an explicit owner: the engine is Rust, the existing lists are TypeScript constants, and app/ cannot reach the indexer's vocabulary at build time. facets.rs is that owner. The tag-support floor exists because a tag on one app can never split the candidate set usefully but still costs a full IG evaluation."
  affects: [1.2]
  check: "grep -n 'CATEGORY_SLUGS\\|CHAIN_SLUGS\\|MIN_TAG_SUPPORT' indexer/src/find/facets.rs indexer/src/find/params.rs"
  closes: []
  status: verified

- id: A36
  phase: 4
  node: 1.7
  route: verifiable
  claim: "Navbar.tsx's mobile-dropdown comment was changed from 'the 4-link nav' to 'the nav', beyond the one-line NAV insertion 1.7.md authorised."
  because: "adding /find makes NAV 5 entries; leaving a hardcoded count in a comment would have shipped a statement that is false on arrival, which this repo's comment discipline forbids."
  affects: [1.7]
  check: "grep -n 'the nav lives in a dropdown' app/src/components/Navbar.tsx"
  closes: []
  status: verified

- id: A37
  phase: 4
  node: 1.7
  route: verifiable
  claim: "findConfirmSchema accepts `turnstileToken` but FindConfirmInput (the indexerClient payload) omits it and requires visitorId/sessionId — so /api/find/confirm consumes the token itself, drops it, and substitutes the server-derived identity before calling recordFindOutcome."
  because: "1.7.md pinned both shapes without stating the transform between them; node 1.8 codes against these two exports and never sees this node, so the asymmetry needed recording rather than inferring. Mirrors api/track/route.ts exactly."
  affects: [1.8]
  check: "grep -n 'turnstileToken' app/src/lib/validation.ts && grep -n 'visitorId' app/src/lib/types.ts"
  closes: []
  status: verified

- id: A35
  phase: 4
  node: 1.1
  route: verifiable
  claim: "params.rs's MIN_CONTENT_GAP doc comment states the A32 reading — LAMBDA bounds the learned term against the answer-conditioned POSTERIOR gap, and the no-flip property is the algebra `content_a - content_b > LAMBDA => order preserved for any support`, not a claim that the scorer emits literally-0.05-separated values."
  because: "node 1.1 first drafted the constant as a hard precondition on the content scorer (facets.rs must emit gaps >= 0.05). A32, written by node 1 while 1.1 was in flight, pins the opposite and better reading: posteriors are continuous so no literal minimum gap exists in the data. The doc comment was rewritten to match, because siblings 1.2/1.4 read this comment as their contract and a wrong one would have propagated silently."
  affects: [1.2, 1.4]
  check: "grep -n -A6 'MIN_CONTENT_GAP' indexer/src/find/params.rs"
  closes: []
  status: verified

- id: A38
  phase: 4
  node: 1
  route: verifiable
  claim: "The run's two workspace-wide Rust acceptance commands are replaced by package-scoped ones: `cargo clippy -p nebulous-world-indexer --all-targets -- -D warnings -> exit 0` and `cargo test -p nebulous-world-indexer -> all green`. The workspace forms are recorded as blocked-by-environment, not as passing."
  because: "`cargo clippy --workspace --all-targets` and `cargo test --workspace` fail at PRE-EXISTING baseline in this worktree: all 12 LiteSVM suites under programs/nebulous_world/tests/ abort with `couldn't read target/deploy/nebulous_world.so`, which only `anchor build` produces and which needs a Solana toolchain this environment does not have. The failure is unrelated to /find. The package-scoped commands cover 100% of the Rust this run touches and are the strongest checks that actually run. An earlier reading of `cargo clippy --workspace ... | tail -20` as exit 0 was wrong — the pipeline reported tail's status, not clippy's."
  affects: [1, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6]
  check: "cargo clippy -p nebulous-world-indexer --all-targets -- -D warnings; cargo test -p nebulous-world-indexer"
  closes: []
  status: verified

- id: A39
  phase: 4
  node: 1.5
  route: verifiable
  claim: "\"FindSession\" carries no foreign key to \"App\", but the migration comment does NOT justify that by precedent, because the precedent 1.5.md cited is false: \"PageView\" DOES have PageView_appId_fkey ... ON DELETE CASCADE (005_app_schema.sql line 361). The justification written instead is the real one — an outcome must survive, and must not fail to be written, when the app row later disappears."
  because: "1.5.md instructed a comment asserting \"PageView also stores a bare appId\". Shipping that would have put a checkably false statement in the schema, which this repo's comment discipline forbids and which would mislead the next reader into thinking FK-lessness is the house pattern. The design choice (no FK) is kept; only its stated reason is corrected."
  affects: [1.5]
  check: "grep -n 'PageView_appId_fkey' indexer/migrations/005_app_schema.sql && grep -n 'unlike \"PageView\"' indexer/migrations/009_find_funnel.sql"
  closes: []
  status: verified

- id: A40
  phase: 4
  node: 1.5
  route: verifiable
  claim: "store::load_learned reads every \"FindSession\" row on every call, with no time window, no LIMIT and no aggregation pushed into SQL."
  because: "no spec; the trivial-SQL/pure-arithmetic split is what makes the aggregation unit-testable with no database (brief criterion 5), and there are zero logged sessions today so any window would be guesswork. Recorded rather than pre-optimized because the right window is a product call about how fast the learned signal should forget. QUANTIFIED at review: the query has no WHERE, so the planner seq-scans and NEITHER index helps — FindSession_appId_idx is not covering (\"outcome\" absent) and FindSession_createdAt_idx is unreachable with no predicate. Cost is linear in total rows and every row crosses the wire, which dominates the scan. Free at 0 rows; noticeable around 1e4-1e5 rows once it runs per-question-per-visitor; clearly broken by 1e6. The fix is a GROUP BY plus a \"createdAt\" window, which FindSession_createdAt_idx was already created to serve, or a cache in ApiState. Per-request cost depends on node 1.6's call site."
  affects: [1.5, 1.6]
  check: "grep -n 'SELECT \"appId\", \"outcome\"' indexer/src/find/store.rs"
  closes: []
  status: verified

- id: A41
  phase: 4
  node: 1.8
  route: verifiable
  claim: "When a deduped answer history exceeds MAX_FORWARDED_ANSWERS (16), toFindNextInput/toFindConfirmInput keep the FIRST 16 by first-appearance order and drop the newest, rather than keeping the most recent 16."
  because: "1.8.md pinned the cap but not which end is dropped. Zod's findNextSchema already rejects >16 answers, so this branch is only reachable via a body that bypassed the schema — defence in depth, not a live path. First-appearance truncation keeps the earliest (most-informative, highest-IG) questions, which is the order the engine itself chose. Revisit only if the question cap ever exceeds 16."
  affects: [1.8]
  check: "grep -n 'slice(0, MAX_FORWARDED_ANSWERS)' app/src/app/api/find/findRequest.ts"
  closes: []
  status: verified

- id: A42
  phase: 4
  node: 1.8
  route: verifiable
  claim: "toFindConfirmInput dedupes and caps its answers exactly as toFindNextInput does, though 1.8.md specified dedupe only for the /next path."
  because: "the confirm body is the same client-held history, and it is what the indexer persists as FindSession.answers and trains on. Storing a path with a facet answered twice would train the learned term on a history the scorer could never have produced, so the two paths must normalise identically."
  affects: [1.8, 1.5]
  check: "grep -n -A6 'export function toFindConfirmInput' app/src/app/api/find/findRequest.ts"
  closes: []
  status: verified

- id: A50
  phase: 4
  node: 1.1
  route: verifiable
  claim: "SMOOTHING_ALPHA is deleted from find/params.rs, overriding the tunable lists pinned in tree/1.md and tree/1.1.md, which both name it. The additive smoothing constant alpha is only meaningful alongside a per-tag empirical prevalence estimate; the design ships a single global TAG_PREVALENCE_PRIOR instead, and facets.rs computes no per-tag counts, so there is nothing for alpha to smooth."
  because: "it had no consumer anywhere in the repo and would never acquire one — an unreachable constant, not a not-yet-wired one. Suppressing it with #[allow(dead_code)] was rejected: a constant allow-listed into existence reads to the next tuner as a knob that does something, which is the specific failure params.rs exists to prevent. Recorded rather than silently dropped because two pinned contracts list it, and Phase 5 would otherwise read those lists as still accurate."
  affects: [1.3, 1.6]
  check: "grep -rn SMOOTHING_ALPHA indexer app  ->  no hits"
  closes: []
  status: verified

- id: A43
  phase: 4
  node: 1.2
  route: must-ask
  claim: "The exact wording of the 16 category questions and the 8 chain labels is authored in indexer/src/find/facets.rs (CATEGORY_PROMPTS, CHAIN_LABELS). `web2` reads \"Does it need to work on the regular web, with no crypto wallet?\" rather than \"Web2\"."
  because: "A10 pinned that phrasing is a constants map and A34 pinned facets.rs as its owner, but nobody wrote the copy. It is product voice, aimed at brief success criterion 1 (a visitor with no crypto fluency): \"Is the category defi?\" is unanswerable for that reader, and \"Web2\" names a concept only an insider holds. Copy is cheap to revise in one array; a wrong reading level is not visible in any test."
  affects: [1.2, 1.9]
  check: "grep -n 'regular web, with no crypto wallet' indexer/src/find/facets.rs"
  closes: []
  status: overridden

- id: A44
  phase: 4
  node: 1.2
  route: verifiable
  claim: "content_score derives from AppDto.rank_score alone, not from separate stakeTotal/viewCount/rankScore bands as brief.md's scoring-inputs list reads."
  because: "rank_score is ALREADY the composite — handlers/engine.rs::compute_rank_score folds vote_weight, stake_total, view_count and a freshness decay into it. Banding those three again alongside it would double-count them and hand the funnel a second, divergent definition of app quality. tree/1.md and A32 both say min-max rank_score, so this reconciles the brief's looser phrasing rather than contradicting the pinned contract."
  affects: [1.2, 1.4]
  check: "grep -n 'pub fn compute_rank_score' -A 14 indexer/src/handlers/engine.rs"
  closes: []
  status: verified

- id: A51
  phase: 4
  node: 1.4
  route: verifiable
  claim: "In blend.rs, a non-finite or negative `support`, and a non-finite `outcome_mean`, are coerced to the neutral prior (zero learned contribution) rather than propagating. `shrunk_learned` therefore always returns a finite value in [0,1]."
  because: "f64::clamp returns NaN for a NaN input, so clamping alone does not sanitize. A NaN score is worse than a wrong one — NaN is unordered against everything, so it silently reorders the shortlist instead of failing. Coercing to zero support is the fail-safe direction: corrupt data can only ever cost an app its learned lift, never grant one, which keeps the anti-farming bound intact."
  affects: [1.2, 1.3, 1.5]
  check: "cargo test -p nebulous-world-indexer find::blend::tests::degenerate_inputs_stay_finite_and_bounded"
  closes: []
  status: verified

- id: A52
  phase: 4
  node: 1.4
  route: verifiable
  claim: "`blend` deliberately does NOT sanitize its `content` argument, unlike the learned inputs. A non-finite posterior propagates."
  because: "`content` in [0,1] is the caller's contract (A32); the scorer owns it. Silently clamping a NaN posterior to 0 would bury a scorer bug under a plausible-looking score, whereas the learned inputs come from stored session rows this module cannot vet. The asymmetry is intentional and documented on the function, because a reader who sees one sanitized and one not will otherwise assume an oversight."
  affects: [1.2, 1.3]
  check: "grep -n -A6 'pub fn blend' indexer/src/find/blend.rs"
  closes: []
  status: verified

- id: A53
  phase: 4
  node: 1.9
  route: verifiable
  claim: "FindFunnel posts via a local 6-line postJson helper rather than importing apiPost from @/lib/txClient, despite txClient being the codebase's established client-side POST helper."
  because: "txClient.ts imports @solana/web3.js at module scope, so reusing apiPost would pull the Solana runtime into a page that must work with no wallet and no Solana anything (A4). The {ok, data} envelope it unwraps is lib/api.ts's, not txClient's, so nothing is duplicated but the four lines of fetch. Measurable: /find first-load JS is 107 kB against /rankings' 226 kB."
  affects: [1.9]
  check: "grep -rn 'web3.js\\|txClient' app/src/components/find app/src/app/find"
  closes: []
  status: verified

- id: A54
  phase: 4
  node: 1.9
  route: must-ask
  claim: "/find renders NO Turnstile widget and sends turnstileToken: null on every confirm. CONSEQUENCE: because 1.8's confirm route 403s when TURNSTILE_SECRET_KEY is set and the token fails (A29), a Turnstile-configured production deploy will reject 100% of /find confirmations — silently, since the client treats the write as telemetry and only toasts."
  because: "no spec said whether the funnel's read-only flow should carry a challenge. I defaulted to no widget: /find must work with no wallet and no infrastructure, and a challenge on a read-only page is friction on the exact step (the 'Is this the one?' confirm) that produces the funnel's only training signal. But that default silently disables the learning loop in production, which is the brief's entire secondary scoring input — so this is a product call, not a detail. Resolve by either mounting a Turnstile widget in components/find/ (my scope) or exempting /find/confirm from the challenge and leaning on the rate limiter plus the A16 bounded-influence cap, which is what actually bounds farming."
  affects: [1.8, 1.9, brief.md secondary scoring input]
  closes: []
  status: superseded  # settled by A56

- id: A55
  phase: 4
  node: 1.9
  route: judgment
  claim: "'Not quite' advances a cursor through the shortlist already in hand (next-best suggestion, no new request) rather than re-querying the engine for more questions. Exhausting the shortlist shows a start-over prompt. Separately, funnelProgress falls back to answers.length whenever result is null, so the bar holds its position mid-request instead of dropping to zero."
  because: "A9 pins that rejection keeps the visitor going to the next-best suggestion and does not end the session, which the local cursor satisfies with no extra round trip; the shortlist is capped at 5 (A20) so it cannot run long. The alternative reading — reject re-enters questioning — would need a wire field the /next contract does not carry."
  affects: [1.9]
  check: "grep -n 'cursor' app/src/components/find/FindResults.tsx"
  closes: []
  status: verified

- id: A56
  phase: 4
  node: 1
  route: must-ask
  claim: "Resolving A54: /find keeps the Turnstile gate on the confirm endpoint and the UI is fixed to satisfy it — FindFunnel renders an invisible Cloudflare Turnstile widget mirroring app/src/components/app/TrafficBeacon.tsx and sends the real token. The confirm route's 403 behaviour (A29) is NOT relaxed."
  because: "node 1.9 correctly found that a Turnstile-configured production deploy would 403 every /find confirmation, silently killing the funnel's only training signal, because the UI sent turnstileToken: null. Of the two fixes, relaxing the 403 would delete a gate the brief explicitly asks for ('Cloudflare Turnstile and the existing fixed-window rate limiter gate the confirm endpoint'), so the widget is the correct side to change. TrafficBeacon is the established precedent for an invisible challenge plus a timeout so an unresolving challenge cannot hang the interaction. With NEXT_PUBLIC_TURNSTILE_SITE_KEY unset (local/simulation) no widget renders, the token is null, and TURNSTILE_SECRET_KEY is also unset, so the route lets it through — the two env vars ship together in app/.env.example."
  affects: [1.8, 1.9]
  check: "grep -n 'turnstile' app/src/components/find/FindFunnel.tsx && grep -n 'NEXT_PUBLIC_TURNSTILE_SITE_KEY' app/src/components/find/FindFunnel.tsx"
  closes: [A54]
  status: verified

- id: A57
  phase: 4
  node: 1.3
  route: verifiable
  claim: "`answer_likelihood(_, Skip)` returns exactly 1.0, which is deliberately OUTSIDE the `[EPS, 1-EPS]` clamp. The clamp test asserts the bound over the six evidential (state, answer) pairs only; for Skip it asserts exactly 1.0."
  because: "1.3.md pinned both `Skip == 1.0` and 'every likelihood is inside [EPS, 1-EPS] for all 9 pairs', which cannot both hold. Skip is resolved as the exception because it is not evidence at all — it is the multiplicative identity, the absence of a factor. Concretely: 1.0 is a BIT-EXACT no-op in log space (ln(1.0) is exactly 0.0, DAMPING*0.0 is exactly 0.0, acc+0.0 is exact), whereas a clamped Skip would add a needless non-identity term to a path both cited precedents (R e1071, Burgener 20Q) implement as a strict no-op. NOTE (corrected in review): clamping would NOT bias the ranking — pre-clamp Skip is 1.0 for all three states, so a clamped Skip returns 1-EPS for Present, Absent and Unknown alike, a common factor that cancels exactly under normalization. An earlier draft of this entry claimed it would penalize Present candidates; that was false. The conclusion stands on exactness and precedent fidelity, not on a behavioural difference."
  affects: [1.3, 1.6]
  check: "cargo test -p nebulous-world-indexer find::scoring  # skip_is_a_strict_no_op_for_every_state + evidential_likelihoods_stay_inside_the_clamp"
  closes: []
  status: verified

- id: A58
  phase: 4
  node: 1.3
  route: verifiable
  claim: "`log_weight` floors the content-score prior at `params::EPS` rather than introducing a new dedicated constant."
  because: "1.3.md says the prior is 'floored so ln is finite' without naming a value, and params.rs belongs to node 1.1 so a new constant cannot be added from here. EPS already carries exactly this meaning in this engine — the floor below which nothing may fall so nothing is ever eliminated — and content_score is min-max normalized, so the worst-ranked app sits at exactly 0.0 and would otherwise get ln(0) = -inf, eliminating it and breaking brief success criterion 2."
  affects: [1.3, 1.4, 1.6]
  check: "grep -n 'content_score.max(params::EPS)' indexer/src/find/scoring.rs"
  closes: []
  status: verified

- id: A59
  phase: 4
  node: 1.3
  route: judgment
  claim: "`expected_information_gain` uses UNDAMPED likelihoods, while `log_weight` applies CORRELATION_DAMPING. The A13 damping is not mirrored into the IG computation."
  because: "1.3.md lines 128-131 pin the IG formula on raw `p(r | q, y_i)` and say nothing about damping there, and the asymmetry is principled rather than an oversight: A13's defect is double-counting of CORRELATED EVIDENCE ACROSS ANSWERS, a property of the accumulated posterior. IG scores a single next answer in isolation, where nothing has yet been counted twice. This is NOT free, and an earlier draft of this entry wrongly claimed it was 'without changing the argmax'. Measured over 200k random (posterior, facet-pair) draws, the reported-IG ordering disagrees with the ordering by true damped entropy drop in ~3% of cases, worst case sacrificing ~0.05 nats; undamped IG over-estimates the damped drop in ~98% of draws. The cost is bounded and lands on question EFFICIENCY only: the deviation picks a slightly less discriminating but still informative facet. It cannot flip 'ask' into 'stop', because MIN_INFORMATION_GAIN is 1e-6 — orders of magnitude below the deviation scale — and a facet with genuinely zero IG scores exactly zero under both. Erring toward over-estimated gain also biases the funnel toward asking one more question rather than stopping early, which is the same conservative direction A13's damping exists to produce."
  affects: [1.3, 1.6]
  closes: []
  status: verified

- id: A60
  phase: 4
  node: 1.3
  route: verifiable
  claim: "The A11 anti-even-split test uses facet A = 5 Present / 5 Unknown (an exact 50/50 mass split) versus facet B = 3 Present / 7 Absent, not the all-Unknown facet A that 1.3.md's test sketch described."
  because: "with every candidate Unknown, facet A's IG is exactly 0 and no posterior mass is Present, so the fixture would neither be a 50/50 split under any reading nor distinguish max-IG from generalized binary search — a GBS implementation would pass it. The Present/Unknown mix makes A the genuine even-split winner (mass 0.5 vs 0.3) while B still wins on IG (0.428 vs 0.387 nats), so the test actually fails against GBS. The all-Unknown case is retained separately as an IG == 0 assertion."
  affects: [1.3]
  check: "cargo test -p nebulous-world-indexer find::selection::tests::max_ig_is_not_the_even_split_rule_a11"
  closes: []
  status: verified

- id: A61
  phase: 4
  node: 1.9
  route: verifiable
  claim: "FindFunnel obtains ONE Turnstile token per page load and reuses it for every outcome in that session, because it reuses the shared `Window.turnstile` global declared in components/app/TrafficBeacon.tsx, which exposes only `render` — not `reset`. A visitor who clicks 'Not quite' and then 'Yes' spends the token on the rejection; Cloudflare rejects the already-redeemed token on the confirm, so that row 403s."
  because: "implements A56 by mirroring TrafficBeacon as instructed. Redeclaring `Window.turnstile` locally with a wider type is a TS duplicate-declaration error ('subsequent property declarations must have the same type'), and TrafficBeacon.tsx is outside node 1.9's six-file scope, so widening the shared global to add `reset(widgetId)` — the real fix — is not mine to make. Impact is bounded: the fallback for a redeemed token is identical to the fallback for a null one (the telemetry row is lost, the visitor's shortlist is untouched), so this is strictly better than the pre-A56 state where EVERY outcome 403'd. Worth fixing when someone owns both files, since 'Not quite then Yes' is a common path and the confirm is the strongest training signal."
  affects: [1.9, learning loop signal volume]
  check: "grep -n 'turnstile.render\\|reset' app/src/components/find/FindFunnel.tsx app/src/components/app/TrafficBeacon.tsx"
  closes: []
  status: superseded  # settled by A62

- id: A62
  phase: 4
  node: 1
  route: must-ask
  claim: "Resolving A61 inside node 1.9's existing scope: FindFunnel.tsx cycles the Turnstile token after every outcome POST by structurally narrowing `window.turnstile` at the call site to reach `reset`, rather than widening the shared `declare global` in TrafficBeacon.tsx. No seam node is created."
  because: "A61 is a BIASED-SAMPLE loss, not attrition — the dropped rows are exactly the 'Not quite, then Yes' sessions, i.e. every session where the content score guessed wrong first and right second. Those carry a correction rather than a confirmation and are the highest-information rows in the training set. Losing them feeds back a set dominated by first-try-correct sessions, entrenching the ranking the learned term exists to counteract (Q6/Q7). That is too serious to defer. But app/src/components/app/TrafficBeacon.tsx is NOT in node 1's scope (tree/1.md frontmatter), so a seam child owning it would violate strict-shrink — a child's scope must be a strict subset of its parent's. A local structural narrowing needs no `declare global` (which would collide with TrafficBeacon's via interface merging) and stays entirely inside app/src/components/find/."
  affects: [1.9]
  check: "grep -n 'reset' app/src/components/find/FindFunnel.tsx"
  closes: [A61]
  status: verified

- id: A63
  phase: 4
  node: 1.9
  route: verifiable
  claim: "The Turnstile token is cycled only when the outcome POST actually redeemed one (turnstileToken non-null), not after every successful POST as A62 literally specified."
  because: "on the tokenless path — no site key, or the 4s timeout expired before a token arrived — there is nothing to burn, and calling reset() would cancel an in-flight challenge that was about to mint the token the NEXT outcome needs. Resetting there would make the timeout fallback self-perpetuating: every outcome would cancel the challenge that would have served its successor. Narrower than A62's wording, same intent."
  affects: [1.9]
  check: "grep -n 'if (turnstileToken) cycleTurnstileToken' app/src/components/find/FindFunnel.tsx"
  closes: []
  status: verified

- id: A64
  phase: 4
  node: 1
  route: judgment
  claim: "Node 1.9 is granted ONE final defect round beyond the 'round 2 of 2' limit I stated to it, making three in total. If this round does not land, 1.9 closes as unfinished and A61 is reported as an open functional gap."
  because: "1.9's first two rounds were defects I raised BEFORE any review (A54, A61); this is the first reviewer-driven round, and the buildteam two-round limit is written against review cycles specifically. The limit exists to stop a child flailing without converging, and 1.9 has converged on exactly what it was asked each time — these are new findings from a first review, not repeats. Recorded rather than applied silently because I did tell the child 'round 2 of 2', and moving that line without saying so would be goalpost-shifting. The alternative — closing now — ships a known functional gap (A62's cycling fires on POST success rather than token redemption), an inert `busy` prop, and a test that cannot fail, when all four defects are small and precisely located."
  affects: [1.9]
  closes: []
  status: verified

- id: A65
  phase: 4
  node: 1
  route: judgment
  claim: "Ledger amendment convention for the rest of this run: a wrong `claim` gets a NEW id superseding the old via `closes:`. A wrong `because` with the `claim` intact is amended IN PLACE with an explicit 'corrected in review' annotation, no new id."
  because: "a supersede chain is for a decision that turned out wrong, not for faulty reasoning behind a decision that stands. Filing superseding entries for a bad `because` would leave the original sitting at status:assumed forever with nothing able to close it — and Phase 5 enumerates by status, so that manufactures exactly the unresolvable entries the ledger exists to prevent. In-place plus annotation keeps the correction legible, which is the property that actually matters. Established when node 1.3 amended A57/A59 this way and asked whether the run required new ids."
  affects: [1.6, 1.9, "any later node"]
  check: "grep -n 'corrected in review' .agent/decisions.md"
  closes: []
  status: verified

- id: A66
  phase: 4
  node: 1.9
  route: verifiable
  claim: "The outcome sequence (acquire token -> POST -> cycle) is a pure injected-dependency function `submitOutcome(deps, answers, appId, outcome)` in funnelState.ts, and the token-cycling tests assert on the recorded request BODIES rather than on the helper return values. The cycle fires in `finally`, so redemption rather than POST success is the trigger."
  because: "the previous A62 test hand-walked receiveToken/spendToken/receiveToken and asserted their return values, so it stayed green even with the component's cycle call deleted — coverage in appearance only, the same inert-fixture failure mode caught in 1.3 and 1.1. Verified by mutation: moving the cycle onto the success path fails 'cycles after a FAILED post' (1 failed), and deleting the cycle entirely fails that plus the A62 two-token test (2 failed | 139 passed). The `finally` placement is required because /api/find/confirm verifies the token BEFORE it writes (route.ts:34-39), so a 403/500 has already consumed it at Cloudflare."
  affects: [1.9]
  check: "cd app && npx vitest run src/components/find/funnelState.test.ts -t 'cycles after a FAILED post'"
  closes: []
  status: verified

- id: A67
  phase: 4
  node: 1.9
  route: verifiable
  claim: "FindResults' `busy` prop is driven by a dedicated `outcomeBusy` state covering the whole recordOutcome window (token wait included), not by `state.loading`."
  because: "the results branch only renders when `state.loading` is already false, so `busy={state.loading}` was a compile-time false and every disabled= on the confirm/reject/restart buttons was inert. Nothing guarded the up-to-4s awaitTurnstileToken wait, so repeated 'Not quite' clicks fired one outcome write per click — repeated writes from a single click-through is anti-farming-adjacent, not merely a UI nit."
  affects: [1.9]
  check: "grep -n 'busy={outcomeBusy}\\|setOutcomeBusy' app/src/components/find/FindFunnel.tsx"
  closes: []
  status: verified

- id: A68
  phase: 4
  node: 1.6
  route: verifiable
  claim: "The indexer exposes a THIRD find route beyond A19's two: GET /find/stats -> {avgQuestionsToConfirm: number|null}."
  because: "store::avg_questions_to_confirm exists for brief success criterion 7 ('average questions-to-confirm is measurable') but had no caller, so the metric was written and never observable — and an uncalled pub fn is a dead_code error under -D warnings, which this node must clear. Giving it a real consumer beats allow-listing it dead in store.rs, which is outside my scope anyway. It is a single aggregate over FindSession and discloses nothing about the catalog, so it does not widen the A6/A20 leak surface. No app/ route consumes it yet; it is an operator/telemetry endpoint."
  affects: [1.6, 1.5, "brief success criterion 7"]
  check: "grep -n 'find/stats' indexer/src/handlers/find.rs"
  closes: []
  status: verified

- id: A69
  phase: 4
  node: 1.6
  route: verifiable
  claim: "blend::preserves_order and params::MIN_CONTENT_GAP get their only non-test consumer in handlers/find.rs — a log::debug of the measured top-two posterior gap on every completed turn — and Candidate.app_id gets one by having the shortlist resolve candidates back to AppDto by id rather than by vector position."
  because: "after wiring, those three were the entire dead_code residue blocking `-D warnings`. All three live in find/*.rs, outside this node's three-file scope, so #[allow(dead_code)] was not available to me even where it would have been acceptable. Both consumers are real rather than linter-appeasing: MIN_CONTENT_GAP's own doc comment asks to be revisited 'if measured posteriors at the stopping point routinely separate by less', and nothing was measuring that; and keying the shortlist by app id removes a silent dependency on facets::candidates_from_apps preserving input order."
  affects: [1.6, 1.4, 1.1]
  check: "grep -n 'preserves_order\\|MIN_CONTENT_GAP\\|by_id' indexer/src/handlers/find.rs"
  closes: []
  status: verified

- id: A70
  phase: 4
  node: 1.6
  route: verifiable
  claim: "A52's guard is implemented as 'non-finite sorts last', not 'non-finite is rejected before sorting': a NaN/inf blended score is ordered behind every finite one and keeps input order among its peers, but is never dropped from the shortlist."
  because: "1.6.md offered either total_cmp or rejection, but rejection can empty the shortlist when every score is corrupt, and brief success criterion 2 says no answer path may. Sorting them last satisfies the real requirement — a corrupt score can never win — without a filter that could empty the list. Plain total_cmp alone was NOT enough either: it orders +NaN ABOVE +inf, so a NaN posterior would have headed the shortlist."
  affects: [1.6]
  check: "cargo test -p nebulous-world-indexer handlers::find::tests::non_finite_scores_sort_last_and_never_scramble_a52"
  closes: []
  status: verified

- id: A71
  phase: 4
  node: 1.6
  route: verifiable
  claim: "POST /find/next enforces the same 16-answer bound 1.6.md specified for /find/confirm, and /find/confirm additionally 400s on an empty appId, visitorId or sessionId."
  because: "the /next path loops per answer over the whole catalog, so an unbounded history is the cheaper denial-of-service of the two, and A41 already caps both app-side paths at 16 — the indexer bound is defence in depth behind the same number. The empty-id rejection exists because A31's idempotency key is (sessionId, appId, outcome): a blank sessionId would collapse every visitor onto one key, so the first confirm would silently suppress everyone else's. NOTE (corrected in review): this entry originally cited confirm_rejects_unknown_outcomes_and_overlong_histories as its check, but that test only ever built non-empty ids and a length derived from MAX_ANSWERS itself, so it exercised neither half of the claim — deleting the sessionId condition and setting MAX_ANSWERS to 100000 both left the suite green. The claim was true; nothing was holding it true. Two dedicated tests now do, and the check field below is the corrected one."
  affects: [1.6]
  check: "cargo test -p nebulous-world-indexer handlers::find::tests::confirm_rejects_an_empty_identity_field handlers::find::tests::answer_history_is_bounded_at_sixteen_on_both_routes"
  closes: []
  status: verified

- id: A72
  phase: 4
  node: 1.6
  route: verifiable
  claim: "The \"App\"/\"AppTag\"/\"Tag\" query in handlers/find.rs is REVIEW-VERIFIED against 005_app_schema.sql, identifier by identifier, and has never been executed. No SQL in this build ever has."
  because: "there is no Postgres in this environment and none may be started. `cargo check` treats a query string as opaque, so it would happily compile a column that does not exist; the runtime sqlx APIs (never the query! macros) give no compile-time schema check either. The 17 App columns, the AppTag/Tag join and every Rust type in the FromRow impl were checked by hand against the migration and against handlers/apps.rs's APP_ROW_COLUMNS, which is the strongest evidence available here and is still not execution. Same standing as node 1.5's store.rs SQL."
  affects: [1.6, 1.5, "run-level gap"]
  check: "grep -n '\"iconUrl\"\\|\"rankScore\"\\|\"submittedBy\"' indexer/migrations/005_app_schema.sql && grep -n 'FROM \"AppTag\" at' indexer/src/handlers/find.rs indexer/src/handlers/apps.rs"
  closes: []
  status: verified

- id: A73
  phase: 4
  node: 1
  route: must-ask
  claim: "Node 1 RATIFIES A68's third route, GET /find/stats -> {avgQuestionsToConfirm}, as an intended part of the wire contract rather than scope creep. tree/1.md's 'two routes' pin is superseded to three."
  because: "brief success criterion 7 requires average questions-to-confirm to be MEASURABLE 'so the self-tuning loop's effect is observable rather than assumed'. Without a route the metric was written but unreachable, i.e. the criterion was satisfied on paper only. Review confirmed the response is a single Option<f64> with no per-app data and no candidate identities, so it is leak-clean under A6/A20, and unauthenticated is consistent with every other indexer route (render.yaml documents the indexer as a private service reached only over the internal network). Ratified explicitly because a node deviating from a PINNED contract should not be able to self-approve, even when the deviation is right."
  affects: [1, 1.6]
  check: "grep -n 'find/stats' indexer/src/handlers/find.rs"
  closes: []
  status: confirmed

- id: A74
  phase: 4
  node: 1
  route: verifiable
  claim: "Mutation-verification must be driven by an enumeration of the GUARDS a file contains, not by a self-selected list of mutations. 'I ran N mutations and all N killed a test' does not imply every guard is covered."
  because: "node 1.6 reported 10 mutations, each of which turned a test red — a true statement — and its review then found two guards with NO test at all (the empty-id rejection and the MAX_ANSWERS value), because those two were simply not among the 10 it chose. Selection bias in the mutation set reproduces the exact inert-test failure this run has now hit four times. The check is to enumerate every branch/constant that encodes a decision and confirm each has a test that dies without it."
  affects: [1.6, "any later node"]
  check: "for each guard in a file, delete it and confirm cargo test goes red"
  closes: []
  status: verified

- id: A75
  phase: 4
  node: 1
  route: judgment
  claim: "Root node interfered with node 1.6's working file: mutated and restored indexer/src/handlers/find.rs while 1.6 was still writing it, because 1.6's node file already read `status: done`. The 4 sort-order test failures observed afterward are attributable to that interference, not to 1.6's work."
  because: "a child sets its own node file status BEFORE returning, so `status: done` in tree/<id>.md is not a signal that the agent has finished — only the return notification is. I treated the file as authoritative and began the final acceptance sweep against a live worktree. Recorded so the failure is not later misread as a defect of 1.6, and so the run's own convention is fixed: never run acceptance, mutation, or any write against a node's scope until that node has actually returned."
  affects: [1, 1.6]
  check: "grep -n 'status:' .agent/tree/1.6.md   # node-file status is set by the child pre-return, not post-review"
  closes: []
  status: superseded  # by A76 — diagnosis wrong, policy conclusion stands

- id: A76
  phase: 4
  node: 1.6
  route: verifiable
  claim: "Correcting A75's attribution: the 4 sort-order test failures the root node observed in find.rs were transient states of node 1.6's OWN guard-mutation battery, which mutates the file in place, runs the suite, and restores. They were not caused by the root node's restore. A75's policy conclusion stands unchanged and is right; only its diagnosis is wrong."
  because: "the reported values identify the mutant exactly: `left: [a0,a1,a2,a3,a4] right: [a6,a5,a4,a3,a2]` is input order, which is what mutant G16 (sort key blend::blend -> raw posterior, giving a uniform posterior and hence a stable no-op sort) produces by construction, and the co-failing score_order/non-finite tests are the G11-G14 comparator mutants. The observed 141->142 test-count drift was tests being added between two of my own runs. Verified after the fact: the current file is byte-identical to the snapshot the battery restores from, all 33 mutants die, and the suite is green — a real corruption would not restore to a passing state. Filed as a new id rather than an in-place amendment because A75's claim, not merely its reasoning, is what is wrong (A65)."
  affects: [1, 1.6]
  check: "cargo test -p nebulous-world-indexer -> 142 passed, 0 failed; cargo clippy -p nebulous-world-indexer --all-targets -- -D warnings -> exit 0"
  closes: [A75]
  status: verified

- id: A77
  phase: 4
  node: 1.6
  route: must-ask
  claim: "Three guards in handlers/find.rs are unguarded by any test and cannot be tested in this environment: (a) `if inserted { bump_facet_stats }` — that a replayed confirm does not inflate the facet tallies; (b) `WHERE status = 'approved'` — that the funnel never scores a pending or rejected app; (c) the three route path strings, so a typo in '/find/next' would only surface at runtime. Shipping them uncovered is a deliberate, recorded choice."
  because: "(a) and (b) need a live Postgres, which this environment has none of and forbids starting. (c) needs an axum request-level harness: `tower::ServiceExt::oneshot` is not reachable because `tower` is not a direct dependency of the indexer crate and Cargo.toml is outside this node's three-file scope, and it would also require constructing a whole ApiState (PgPool, RpcClient, three Pubkeys). Applying A74 honestly means naming these rather than reporting 100% guard coverage: I enumerated 33 guards the file contains and killed all 33, but that enumeration covers only what is reachable without a database. Closing these needs an integration-test tier this run does not have."
  affects: [1.6, 1.5, "post-run verification"]
  closes: []
  status: overridden

- id: A78
  phase: 5
  node: 1.10
  route: verifiable
  claim: "Every /find question is NEED-framed, not object-framed: all 16 CATEGORY_PROMPTS, the chain template (\"Do you need it to work on {label}?\"), the tag template (\"Do you want something to do with \\\"{tag}\\\"?\") and both unknown-slug fallbacks now open with \"Do you \". A78 supersedes A43, which the user overrode in Phase 5. A new test, a78_every_prompt_asks_about_the_need_not_the_app, asserts the opening over every category, every chain and all three fallback paths."
  because: "the user's override: /find is a GUIDED DISCOVERY funnel, so the visitor has a need and NO app in mind. A43's mixed framing asked most questions as \"Is it a game?\" — which can only be answered by describing the thing the visitor came here to find, i.e. unanswerable by construction — while defi and ai already asked \"Are you looking to ...?\". The user supplied defi/nft/gaming/dao/wallet verbatim as the voice reference; the other 11 are written to match, still jargon-free per PRODUCT.md and still short enough to read as one tappable question. The chain and tag templates were converted too, so the funnel does not change register mid-session. Encoded as a test rather than a comment because a reading level is invisible to every other check in this repo, which is exactly why A43's split framing survived a full review round."
  affects: [1.2, 1.9, "product voice"]
  check: "cargo test -p nebulous-world-indexer find::facets::tests::a78_every_prompt_asks_about_the_need_not_the_app"
  closes: [A43]
  status: verified

- id: A79
  phase: 5
  node: 1.10
  route: verifiable
  claim: "indexer/src/find/facets.rs was run through rustfmt in full, which also reformatted two regions untouched by the copy edit: CHAIN_SLUGS collapsed to one line, and the min/max `fold` in candidates_from_apps re-wrapped. The file was already fmt-dirty at baseline in both places."
  because: "the fix task's acceptance is `cargo fmt --check` -> no diff in facets.rs, and the new copy cannot satisfy rustfmt's 60-char fn_call_width for tuple literals without shortening the five prompts the user gave verbatim. Formatting the file was the only way to meet the acceptance without editing approved copy. Recorded because the two collateral hunks are outside the stated intent of this task, though inside its scope. Note `cargo fmt --check` is still non-zero repo-wide from PRE-EXISTING diffs in the autogenerated indexer/decoder crate, unrelated to /find."
  affects: [1.2]
  check: "rustfmt --edition 2021 --check indexer/src/find/facets.rs -> exit 0, no output"
  closes: []
  status: verified

- id: A80
  phase: 5
  node: 1.11
  route: verifiable
  claim: "Resolving the A29 override: \"FindSession\" gains a `turnstileVerified` BOOLEAN NOT NULL DEFAULT true; /api/find/confirm records the outcome with that flag instead of returning 403 on a failed token; store::load_learned filters to verified rows so unverified outcomes never reach the learned weights. The app-side gate is `mayRecordOutcome(configured, verified) = !configured || verified`."
  because: "user overrode A29 in Phase 5. A refused write is not a neutral loss — Turnstile fails hardest for VPN users, privacy browsers and slow connections, so dropping those rows biases the training set toward one kind of visitor AND makes the drop rate unmeasurable. Recording with a flag buys farming nothing (load_learned filters) while leaving the loss countable. DEFAULT true is reachable only by a writer with no notion of Turnstile at all — local/simulation, where nothing is configured and there is nothing to verify; store::record_outcome always binds the flag explicitly, so the configured-and-failed case never falls through to it. Defaulting to false would leave load_learned returning an empty map for every local run, killing the learning loop exactly where A4 requires the funnel to work with no infrastructure."
  affects: [009_find_funnel.sql, store.rs, handlers/find.rs, confirm/route.ts]
  check: "grep -n turnstileVerified indexer/migrations/009_find_funnel.sql && grep -n 'mayRecordOutcome' app/src/app/api/find/findRequest.ts && cd app && npx vitest run src/app/api/find/findRequest.test.ts -t mayRecordOutcome"
  closes: [A29]
  status: verified

- id: A81
  phase: 5
  node: null
  route: verifiable
  claim: "Ledger id A78 was claimed simultaneously by three Phase 5 nodes (1.10 need-framing, 1.11 turnstileVerified, 1.12 URL mirror). 1.10 owns A78 in the ledger; 1.11's citations were renumbered to A80 across 5 files; 1.12's are renumbered to A82 once that node returns (A81 is this record itself)."
  because: "my error, not the nodes'. I told all three fix tasks 'the highest currently used is A77' in the same message, which guaranteed a collision the moment they ran in parallel. The A65 re-read-before-appending convention cannot help when the reads are concurrent. Node 1.9 predicted exactly this reaching into code comments, where a stale id is invisible to any check that only reads decisions.md."
  affects: [decisions.md, 009_find_funnel.sql, store.rs, handlers/find.rs, confirm/route.ts, confirm/route.test.ts]
  check: "test $(grep -rho 'A78' indexer/src/find/facets.rs | wc -l) -eq 3 && ! grep -rq A78 indexer/src/find/store.rs indexer/src/handlers/find.rs indexer/migrations/009_find_funnel.sql"
  closes: []
  status: verified

- id: A82
  phase: 5
  node: 1.12
  route: verifiable
  claim: "Resolving the A1 override: the answer history is mirrored into the query string as `/find?a=category:defi:yes,tag:lending:no` — one entry per answer, `kind:value:answer`, `:` and `,` left literal and only the facet value percent-encoded. Restoring is validated by parseFunnelAnswers, which TRUNCATES at the first entry that does not parse rather than skipping it, then applies the route's own dedupeAnswers and MAX_FORWARDED_ANSWERS=16 (A41). All of mount, refresh, browser Back and browser Forward run through one injected-dependency seam, reconcileUrlAnswers(deps, raw, state). Answering pushes a history entry; the in-page Back pops one only when the session pushed it (backNavigation), and rewrites in place at a shared link's own depth. The server stays stateless — no session table, no schema, no indexer change."
  because: "user overrode A1's consequence in Phase 5: as built, a refresh lost the funnel and a path could not be shared. Legibility is a requirement, not taste — these URLs get pasted to other people, so an opaque blob would be a worse artifact than the state it encodes. Truncate-rather-than-skip is the one non-obvious call: a query string that stops parsing was most likely cut short in transit, and its sound prefix is a state the funnel genuinely passed through, whereas stitching the far side of the damage back on assembles an answer path nobody walked and then trains the learned term on it. backNavigation exists because router.back() on a shared link's first state pops an entry belonging to whoever shared it, i.e. leaves the site — the exact bug this mirror exists to fix, in reverse. /find/page.tsx also skips its server-side first-question fetch when the URL is resuming: every answered turn is now a push against a force-dynamic route, so that fetch would otherwise re-ask the indexer for question 1 on every answer. A5 is now load-bearing rather than theoretical and is CONFIRMED to cover the new strings by a test on generateMetadata itself (?a=... -> index:false, follow:true, canonical /find), not by reading isParameterized. A53 holds: /find first-load JS is 110 kB, unchanged, against /rankings' 226 kB."
  affects: [1.9, 1.12, SEO, "shared funnel links"]
  check: "cd app && npx vitest run src/components/find/funnelState.test.ts -> 59 passed"
  closes: []
  status: verified

- id: A83
  phase: 5
  node: 1.12
  route: must-ask
  claim: "Three things node 1.12 added are guarded by NO test and cannot be in this environment: (a) that FindFunnel's one URL effect actually calls reconcileUrlAnswers, (b) that handleAnswer pushes / handleBack pops via the Next router at all, and (c) page.tsx's `resuming` short-circuit of the server-side first-question fetch. The 30-mutant guard battery covers the pure layer those three feed, and nothing else. Second id taken by 1.12 beyond A81's allocation of A82."
  because: "app/ has no jsdom and no @testing-library — vitest runs in the node environment and package.json is outside this node's four-file scope, so no component may be rendered and no router mocked. Applying A74 honestly means naming this rather than reporting the battery as full coverage: every DECISION was lifted out of the component into funnelState.ts precisely so it could be mutated, but the wiring that reaches those decisions is exactly what A66 caught the previous node shipping untested. Closing it needs a component-test tier (jsdom + @testing-library/react + next/navigation mocks) this run does not have."
  affects: [1.12, "post-run verification"]
  closes: []
  status: overridden
  resolution: "User folded this into the A77 integration tier rather than adding a jsdom/@testing-library DOM tier to app/. One tier, exercised through a real HTTP/browser path, instead of two."

- id: A84
  phase: 5
  node: 1.13
  route: verifiable
  claim: "The indexer crate has NO library target — Cargo.toml declares only `[[bin]] indexer`. So `indexer/tests/*.rs`, the location the fix task named, CANNOT reach `handlers::find::routes()`, `find::store::*` or `sqlx::migrate!()` against the real modules: an integration test binary links the lib target, and there is none. The A77 tier is therefore built as a `#[cfg(test)] mod find_integration;` INSIDE the bin crate, costing one cfg-gated line in main.rs, rather than adding a `[lib]` target."
  because: "the two alternatives are both worse. Adding `[lib]` while main.rs keeps its own `mod` declarations compiles every module twice and runs all 150 existing unit tests twice, in two targets — the acceptance number stops meaning anything. Converting main.rs into a thin bin over the lib is the idiomatic fix but is a structural refactor of a production binary, far outside a test task's remit and outside this task's stated scope. A `#[cfg(test)]` module compiles to nothing in release, adds no target, and reaches the private `mod find;`/`mod handlers;` tree because it is in the same crate. The deviation from the task's `indexer/tests/` path is recorded rather than silently taken."
  affects: [1.13, 1.13.1, "indexer/Cargo.toml", "indexer/src/main.rs"]
  check: "! grep -q '^\\[lib\\]' indexer/Cargo.toml && grep -q 'mod find_integration' indexer/src/main.rs"
  closes: []
  status: verified

- id: A85
  phase: 5
  node: 1.13
  route: must-ask
  claim: "Ledger ids are pre-allocated in DISJOINT ranges before the two children of 1.13 run: 1.13.1 (Rust DB/router tier) owns A86-A89, 1.13.2 (app end-to-end tier) owns A90-A93. Neither child may take an id outside its range, and neither re-derives 'the next free id' from decisions.md."
  because: "A81 recorded three nodes colliding on A78 because they were told the same next-free id concurrently, and the A65 re-read-before-appending convention provably cannot help when the reads are concurrent. Pre-allocation is the only thing that survives parallel children. Ranges are wide enough (4 each) that neither child has to ask for more; an unused id is a cheap gap, a collided one costs a rename across files."
  affects: [1.13.1, 1.13.2, "decisions.md"]
  closes: []
  status: confirmed

- id: A86
  phase: 5
  node: 1.13.1
  route: verifiable
  claim: "A77 is CLOSED by execution, not by argument: all three guards it named untestable now have live-Postgres tests, and no SQL defect exists in indexer/src/find/**, indexer/src/handlers/find.rs or migration 009. Migration 009, the 17-column \"App\" SELECT, the \"AppTag\" JOIN \"Tag\", gen_random_uuid()::text, AVG(...)::double precision and both ON CONFLICT targets all executed correctly on Postgres 15.15 on the first attempt."
  because: "the review-verification A72 recorded was right. 9 #[ignore]d tests in indexer/src/find_integration.rs create a per-test find_it_<unique> database, run sqlx::migrate!(\"./migrations\") through the same macro call db.rs uses at startup, and drive the FULL api::router via tower::ServiceExt::oneshot with the real path strings. The only failure execution surfaced was in the test fixture, not the product code (see A88). 22 guard mutations over 21 guards, 22 killed, 0 survivors (G20 and G21 added in review — the key's ARITY and the AnswerValue-to-column mapping, neither of which G2/G3/G12 state)."
  affects: [1.13.1, 1.6, 1.5, "run-level gap"]
  check: "bash app/scripts/find-test-db.sh && cd indexer && FIND_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres cargo test -p nebulous-world-indexer -- --ignored"
  closes: [A77, A72]
  status: verified

- id: A87
  phase: 5
  node: 1.13.1
  route: verifiable
  claim: "The `git diff --exit-code` half of this node's acceptance is VACUOUS as written and must not be trusted as the mutation battery's restore proof. indexer/src/find/**, indexer/src/handlers/find.rs and indexer/migrations/009_find_funnel.sql are all still UNTRACKED in this worktree, so `git diff` over them exits 0 whatever their contents — including if the battery left them mutated. The real proof is a sha256 comparison against a pre-battery snapshot, which the battery now runs and which passed for all four touched files."
  because: "the whole /find feature is uncommitted; `git ls-files --others` lists exactly those three paths. A75/A76 already cost this run a false corruption diagnosis over the same mutate-and-restore mechanism, so the restore check has to actually be able to fail. Kept the git command too (it does cover indexer/src/api.rs, which IS tracked and whose .merge line G9 removes) but the byte comparison is what settles it."
  affects: [1.13.1, "any later mutation battery in this run"]
  check: "git ls-files --others --exclude-standard -- indexer/src/find indexer/src/handlers/find.rs indexer/migrations/009_find_funnel.sql  # non-empty => git diff cannot see them"
  closes: []
  status: verified

- id: A88
  phase: 5
  node: 1.13.1
  route: verifiable
  claim: "tower is pinned at `0.5` in indexer/Cargo.toml's new [dev-dependencies], not the `0.4` the node file suggested: axum 0.7.9 already locks tower 0.5.3, and Cargo.lock still contains exactly one `name = \"tower\"` entry after the change."
  because: "the node file itself required matching whatever axum pulls rather than the literal 0.4, so the lock file does not gain a duplicate major. Recorded because the deviation is from a pinned instruction, and because the added [dev-dependencies] section is the only Cargo.toml change — [dependencies] is untouched, so no runtime dependency was added and the release binary is unchanged."
  affects: [1.13.1, "indexer/Cargo.toml", "Cargo.lock"]
  check: "grep -c 'name = \"tower\"$' Cargo.lock  # -> 1, and `git diff Cargo.lock` is the single line + \"tower\","
  closes: []
  status: verified

- id: A89
  phase: 5
  node: 1.13.1
  route: verifiable
  claim: "Counting a tally only at the END of a replay test cannot distinguish `if inserted` from `if !inserted` — both bump exactly once. The A77(a) test therefore asserts the facet counts after EACH of the two confirms, not only after both. This was caught live: the first battery round mutated `if inserted` to `if !inserted` and the tier stayed GREEN."
  because: "this is A74's failure mode reproduced inside the node written to close A74, which is exactly how cheap it is to write an inert assertion. The final-total assertion is a claim about the sum; the guard is a claim about WHICH call bumps, and only a per-attempt assertion states that. After the fix both mutants (unconditional, and inverted) kill the test. CORRECTED IN REVIEW (A65, reasoning only — the claim above stands and was re-confirmed): the original `check:` ended \"then git checkout the file\", which cannot restore indexer/src/handlers/find.rs. That file is UNTRACKED (this is A87's whole point, and the check contradicted it), so `git checkout` errors on the pathspec and leaves the `sed -i` mutation permanently in a protected file. A verification step that corrupts what it verifies is worse than no check. The battery below mutates and restores from a sha256-checked snapshot instead."
  affects: [1.13.1, "any test that asserts an accumulated count"]
  check: "python3 .agent/1.13.1-guard-battery.py  # G1a and G1b both report KILLED, and every protected file reports sha256 OK afterwards"
  closes: []
  status: verified

- id: A90
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "The /find end-to-end tier runs against e2e/stubIndexer.ts, a ~150-line node server speaking the three raw indexer shapes /find needs, not against the real indexer. It returns a different facet per turn (so a spec can tell question 1 from question 2 by reading the screen) and RECORDS every request, served back at GET /__requests — that recorder is what turns 'did the server fetch question 1?' into an assertion. The whole suite needs Node and a chromium binary: no database, no Solana RPC, no cargo build, no secrets."
  because: "the real binary cannot boot here — indexer/src/main.rs requires a live Solana RPC with an initialised Config account. A83's three gaps are app-side WIRING (does the effect call reconcileUrlAnswers, does handleAnswer reach the router, does page.tsx skip its fetch), and none of them is a claim about what the engine answers, so the indexer contract is out of frame; 1.13.1 covers it against a real database in parallel. Stubbing also buys the thing a real indexer could not: a request recorder, without which the `resuming` short-circuit is unobservable from outside."
  affects: [1.13.2, "app/e2e", "CI"]
  check: "cd app && npm run test:e2e   # passes with no indexer, database or RPC running"
  closes: []
  status: verified

- id: A91
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "The tier runs `next dev`, not `next build && next start`, and its specs are named `*.e2e.ts` rather than `*.spec.ts`. Playwright's own webServer owns both the stub and the Next server; E2E_EXTERNAL_SERVERS=1 suppresses that so the mutation battery can keep one hot-reloading dev server across all mutants. Cost: one devDependency, @playwright/test 1.62.0, so app/package.json and app/package-lock.json both change."
  because: "/find is force-dynamic and is rendered per request under either server, so `start` buys nothing here while costing a full build of ~40 other routes, several of which prerender against the indexer this tier replaces with a stub. Dev also hot-reloads, which is the only reason an 18-mutant battery finishes in minutes rather than restarting a server 18 times. The naming is not cosmetic: vitest's default include is `**/*.{test,spec}.?(c|m)[jt]s?(x)`, so a file named `.spec.ts` under app/e2e WOULD be collected by `npm test` and fail outside a Playwright runner — the task text's suggestion to name them `*.spec.ts` is wrong, and `.e2e.ts` keeps the suites apart without editing vitest.config.ts (which is outside this node's scope). Dev's one cost is React StrictMode's double effect invocation, so the specs assert on request CONTENT and on exact ZERO counts, never on an exact positive count."
  affects: [1.13.2, "app/package.json", "app/package-lock.json", "app/vitest.config.ts (deliberately untouched)"]
  check: "cd app && npm test   # 179 passed — the e2e specs are not collected"
  closes: []
  status: verified

- id: A92
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "This node's stated acceptance check `git diff --exit-code -- app/src` is INERT for exactly the files its mutation battery touches, and was replaced by a byte-level checksum of the whole app/src tree taken before the first mutant and re-checked after the last. Every /find source file is still UNTRACKED in this worktree, so `git diff` cannot observe a mutation to FindFunnel.tsx or find/page.tsx at all; and app/src has five tracked files already modified by earlier nodes, so the check is red before the battery starts and stays red however it ends."
  because: "the check exists to catch a battery that fails to restore, and a check that cannot see the mutated files would have reported a clean restore over an arbitrarily corrupted FindFunnel.tsx. `find src -type f | xargs shasum | shasum` sees untracked and tracked files alike and is indifferent to the worktree's pre-existing dirt. Recorded rather than silently substituted because the acceptance line, taken literally, is unsatisfiable in this worktree for reasons that have nothing to do with this node's work."
  affects: [1.13.2, 1.13, "any later node that mutates app/src before the run is committed"]
  check: "git ls-files app/src/components/find/FindFunnel.tsx   # prints nothing: untracked, so git diff is blind to it"
  closes: []
  status: verified

- id: A93
  phase: 5
  node: 1.13.2
  route: must-ask
  claim: "18 guards enumerated across FindFunnel.tsx and find/page.tsx, 16 killed by the e2e tier, 2 survive and cannot be killed at this tier: G17, handleAnswer's `dispatch({type:'answered'})`, is pure redundancy against the URL — deleting it leaves the router.push, and the effect restores the same history from the query string, so the only thing lost is one frame in which the visitor does not stare at the question they just answered. G18, handleBack's `if (step === 'none') return;`, is unreachable through the UI, because QuestionCard renders Back only when canGoBack (>= 1 answer) and backNavigation returns 'none' only at 0 answers. No defect was found in app/src: every one of the 11 specs passes against unmutated code."
  because: "A74's rule is that a guard which does not die is reported, not papered over, and that the enumeration must come from the file rather than from a self-selected mutation list — so the 11 guards this node was handed were extended with 7 more found by reading the two files (G12 the param key FindFunnel reads, G13 the effect's dependency array, G14 initialResult seeding the reducer, G15 handleAnswer's own fetch, G16 handleBack's slice, plus G17/G18). The battery encodes both survivors as an EXPECTED_SURVIVORS set compared BOTH ways, so a new survivor fails the script and an expected survivor that starts dying fails it too rather than quietly going stale."
  affects: [1.13.2, 1.12, "app/e2e/mutants.py"]
  check: "cd app && bash e2e/mutation-battery.sh   # guards enumerated: 18   killed: 16   survived: 2  [ G17 G18 ]"
  closes: [A83]
  status: superseded  # by A96 — the G17 half of the claim is false, not merely thin
  check_result: "FAILED on re-run by the top-level session. The battery halts with 'BASELINE FAILED - a battery on a red suite proves nothing'. Its 18/16/2 counts are NOT currently reproducible."

- id: A94
  phase: 5
  node: null
  route: verifiable
  claim: "The /find Playwright e2e tier is FLAKY, not green. Four consecutive runs of the same unmutated tree gave 11/11 pass, 10 pass + 2 fail, 12/12 pass, and 7 pass + 5 fail (the battery's own baseline). Failures cluster in e2e/find-history.e2e.ts - history/Back depth assertions and 'an answer costs exactly one engine round trip', which counts requests and so is inherently racy."
  because: "found by the top-level session re-running A93's check rather than accepting the node's reported 18/16/2. A flaky suite is worse than a failing one: it produces false confidence, it cannot gate a merge, and it makes the mutation battery structurally unusable because a flake is indistinguishable from a kill. The battery is right to refuse a red baseline; the tier underneath it has to be made deterministic before its numbers mean anything."
  affects: [A93, A83, "app/e2e", 1.13.2]
  check: "cd app && for i in 1 2 3; do npm run test:e2e 2>&1 | grep -E '^ +[0-9]+ (passed|failed)'; done   # all three runs must be identical and fully green"
  closes: []
  status: superseded

- id: A96
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "G17 — deleting handleAnswer's `dispatch({type:'answered'})` — is NOT unobservable at this tier, and A93 was wrong to record it as a permanent survivor. It costs a second POST /find/next per answer, not merely a frame: reconcileUrlAnswers then finds `restored` and `state.answers` unequal (funnelState.ts:308-312) and fires `request` on top of handleAnswer's own requestNext, against a force-dynamic route. The stub's recorder sees this directly. A new spec, 'an answer costs exactly one engine round trip', asserts the recorded answer-counts are exactly [0,1,2] after two answers; under G17 they are [0,1,1,2,2]. 18 guards enumerated (G1-G18), 17 killed, 1 documented survivor (G18, unreachable because QuestionCard renders Back only when canGoBack) — corrected in place from an arithmetic slip in this same entry, which said 19/18; the measured battery output is 18/17/1."
  because: "A93 reasoned from what the UI shows rather than from what the tier can measure, then hardcoded that reasoning into EXPECTED_SURVIVORS — which asserted the hole was permanent and would have kept any future run from noticing. The guard's real content is the ordering comment in FindFunnel.tsx:177-180: the reducer moves BEFORE the push precisely so the effect finds the two in agreement and does not re-ask, i.e. it is a request-count guard wearing a rendering rationale. A65: the claim itself is false, so this takes a new id rather than amending A93 in place. ID COLLISION: 1.13.2's reviewer allocated it A94/A95 as unheld, but the top-level session had concurrently taken A94 (and this file then had two). Theirs is older and is left alone; these two moved up to A96/A97 rather than clobbering it — A85's pre-allocation only survives concurrency if one allocator holds the whole space, and here two did."
  affects: [1.13.2, 1.12, "app/e2e/mutation-battery.sh", "app/e2e/find-history.e2e.ts"]
  check: "cd app && python3 e2e/mutants.py apply G17 && npx playwright test -g 'exactly one engine round trip'; python3 e2e/mutants.py restore   # 1 failed"
  closes: [A93]
  status: verified

- id: A97
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "The e2e tier hands every test its own synthetic client address via Playwright `extraHTTPHeaders: {'x-forwarded-for': '10.<run>.<n>.<n>'}`, because otherwise the whole suite — every test, every run, forever — shares ONE rate-limit bucket. POST /api/find/next calls requireRateLimit(RATE_LIMITS.read) = 60 per 60s keyed `read:ip:<clientIp>`, and under `next dev` there is no proxy, so clientIp() finds neither x-forwarded-for nor x-real-ip and returns the literal string 'unknown' (lib/api.ts:89-95). A suite run spends ~20 of the 60, so a third or fourth run inside one window rendered 'Too many requests — try again in Ns' where a spec expected a question. clientIp() reads x-forwarded-for FIRST, which is what makes the header the precise fix. An afterEach additionally fails any test that leaves the funnel's error alert on screen, so a 429 can never again be misread as a dead mutant. resetStub() also drains 250ms before clearing, so a straggling request from the previous test cannot land inside the next test's negative assertion."
  because: "the tier was non-deterministic — 2 of 8 consecutive full-suite runs red against unmutated code — and that made the mutation battery's verdicts a coin flip, since it deliberately shares one dev server across all 20 runs. Retries or sleeps would have been the wrong fix twice over: a tier that passes by waiting out a window still cannot tell a mutant from a 429, and the battery specifically needs a red run to MEAN the mutant. Correction to the reported diagnosis: /api/auth/me does not rate-limit at all (no requireRateLimit call), so /find/next is the bucket's only consumer and the limit falls on the third or fourth back-to-back run rather than the second — the root cause and the fix are unchanged."
  affects: [1.13.2, "app/e2e/fixtures.ts", "app/e2e/stub.ts"]
  check: "cd app && for i in 1 2 3 4 5 6 7 8 9 10; do npm run test:e2e; done   # 12 passed x10, no 'Too many requests'"
  closes: [A94]
  status: verified

- id: A98
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "OUT-OF-TEST CONSEQUENCE of the clientIp() fallback, asked for by the coordinator. The 'unknown' fallback itself does NOT bite in production: nebulous-world is a Render `type: web` service (render.yaml:42-51), so it sits behind Render's load balancer, which sets x-forwarded-for on every request — the fallback is reached only if that header is absent, i.e. direct-to-instance traffic or a future proxy change. The sharper finding is the opposite one: clientIp() takes `x-forwarded-for.split(',')[0]`, the FIRST hop, which is the value the CLIENT supplied and which a reverse proxy appends its observed peer IP to rather than replacing. So the anonymous rate limit is evadable by sending a different x-forwarded-for per request — and this node's own test fixture is a working demonstration, since giving each test a synthetic address gave each a fresh bucket. Secondary: lib/api.ts's clientIp() and lib/tracking.ts's clientIpFromHeaders() disagree — tracking also honours cf-connecting-ip and falls back to '0.0.0.0', api.ts honours neither and falls back to 'unknown'. NOT FIXED: app/src is out of this node's scope, and this is reported, not repaired."
  because: "the coordinator asked only whether the 'unknown' fallback applies in production; reading api.ts:89-95 to answer that surfaced the larger issue in the same four lines, and reporting only the narrower answer would have left it invisible. Taking the first hop is right for 'who is the user' (tracking) and wrong for 'whom do I throttle', because for the latter the field is attacker-controlled — the standard fix is to count from the right by a known trusted-proxy depth. The availability half is real but conditional; the evasion half needs no misconfiguration at all. Both are stated so a human can decide, since the failure modes point in opposite directions: fixing evasion by trusting the last hop would break any legitimate multi-proxy deployment."
  affects: ["app/src/lib/api.ts", "app/src/lib/tracking.ts", "render.yaml", "post-run verification"]
  check: "for i in $(seq 1 61); do curl -s -o /dev/null -w '%{http_code} ' -H \"x-forwarded-for: 9.9.9.$((RANDOM%250+1))\" -X POST https://<deployed-host>/api/find/next -H 'content-type: application/json' -d '{}'; done   # all 200 = limit evaded; a 429 appears = header not trusted"
  closes: []
  status: verified
  resolution: "ACTED ON by another session while this node was closing out, not by 1.13.2: lib/api.ts now has throttleIdentityFromHeaders() taking the LAST x-forwarded-for hop at a trusted-proxy depth of one, with lib/api.test.ts (4 tests) covering it. The claim stands as recorded; only the 'NOT FIXED' disposition is superseded. Two consequences for this node, both verified rather than assumed: (1) the e2e tier is unaffected — under `next dev` there is no proxy, so the fixture's header has exactly one hop and first==last, per-test bucket isolation survives (12 passed, 0 429s against the new api.ts); (2) `npm test` is now 183, not the 179 this node's acceptance names — 179 + 4 from their api.test.ts, and e2e specs are still collected 0 times by vitest."

- id: A99
  phase: 5
  node: null
  route: verifiable
  claim: "app/src/lib/api.ts's clientIp now takes the LAST x-forwarded-for hop, not the first, closing the evasion half of A98. The pure part is extracted as `throttleIdentityFromHeaders(headers: Headers)` so it is unit-testable without Next internals; clientIp(req) delegates to it. Empty/whitespace hops are filtered so a header like ' , ' cannot yield an empty bucket key. Fallback order (x-real-ip, then the literal 'unknown') is unchanged and deliberately fails closed."
  because: "user chose in Phase 5 to fix this in-branch rather than defer it, after 1.13.2 found it (A98). A reverse proxy APPENDS its observed peer rather than replacing the header, so index 0 is caller-supplied and every IP-keyed limit in RATE_LIMITS was evadable by varying the header per request. Render is the single trusted proxy (render.yaml type: web), so depth is one and the rightmost entry is the only vouched-for value. Trusted-proxy depth is a deployment property, not a constant — the doc comment says so, because putting Cloudflare or a second LB in front shifts the trusted index and this must move with it."
  affects: ["app/src/lib/api.ts", "all 25 routes calling requireRateLimit", A98]
  check: "cd app && npx vitest run src/lib/api.test.ts   # 4 passed; reverting hops[hops.length-1] to hops[0] fails 3 of them"
  closes: []
  status: verified

- id: A100
  phase: 5
  node: null
  route: must-ask
  claim: "app/src/lib/tracking.ts's clientIpFromHeaders was deliberately NOT changed, so it still reads the first hop and still diverges from api.ts (it also honours cf-connecting-ip and falls back to '0.0.0.0'). The two functions now use different rules on purpose."
  because: "the divergence is defensible — tracking answers 'which visitor is this', where forging a value costs the forger their own identity, while throttling answers 'whom do I throttle', where the field is adversarial. But changing tracking would alter visitorId derivation for every existing visitor, resetting PageView dedup and the revenue-eligibility identity mid-flight. That is a data consequence, not a code cleanup, and it is not mine to take unasked."
  affects: ["app/src/lib/tracking.ts", "PageView dedup", "ad-revenue eligibility"]
  closes: []
  status: assumed

- id: A101
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "HIDDEN COUPLING: the e2e tier's rate-limit isolation depends on clientIp()'s hop-SELECTION rule in app/src/lib/api.ts, and nothing in either file references the other. e2e/fixtures.ts gives each test a synthetic `x-forwarded-for` to get it its own RATE_LIMITS.read bucket. That works under the current last-hop rule only because `next dev` has NO proxy in front: the header therefore carries exactly one hop, so first == last and either rule picks the fixture's value. The coincidence breaks the moment (a) a proxy is put in front of the dev server (docker-compose, a tunnel, a CI service container, anything that appends a second hop — the fixture's value stops being last and every test collapses back onto ONE bucket), or (b) the trusted-proxy depth changes, or (c) clientIp() stops honouring the header at all (e.g. moving to a raw socket IP), which kills the isolation outright. In every case the symptom is 'Too many requests' surfacing in the funnel's error alert on the 3rd-4th consecutive run and looking exactly like fresh Playwright flakiness."
  because: "A97 fixed the tier's non-determinism by leaning on a production rule it does not own and cannot see change. A98's fix landed hours later and silently altered that rule from first-hop to last-hop; the tier survived by luck of the environment, not by design, and I only noticed because a checksum I happened to hold moved. That is precisely the fact that rots: the next person to touch either file has no way to learn the other exists. Recorded on the node side and pinned with a comment in fixtures.ts pointing here, because an entry nobody is routed to is only marginally better than no entry. The afterEach that fails on a leftover funnel error alert is the safety net if this does regress — it turns the 429 into a named failure rather than a mystery 'element not found'."
  affects: ["app/e2e/fixtures.ts", "app/src/lib/api.ts", 1.13.2, "any future proxy in front of the dev server"]
  check: "cd app && grep -n 'A101' e2e/fixtures.ts && grep -n 'hops\\[hops.length - 1\\]' src/lib/api.ts   # both must be present; if the second is gone, re-read fixtures.ts's assumption"
  closes: []
  status: verified

- id: A102
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "e2e/mutants.py and e2e/mutation-battery.sh now take an exclusive lock (mkdir on ${TMPDIR}/find-e2e-mutation.lock, owner pid in owner). The guard lives in mutants.py, not only in the battery script: apply AND restore refuse with exit 1 and name the holding pid, the battery refuses to start rather than beginning a 26-minute run, a run's own child calls pass via FIND_E2E_LOCK_OWNER, a dead owner is auto-cleared as stale rather than deadlocking, and --force is the documented override. Release is in the battery's existing cleanup() trap so it survives a kill."
  because: "two agents ran a battery against FindFunnel.tsx concurrently and silently corrupted each other's verdicts — A75 for the third time in this run. The decisive detail is WHERE the guard goes: the call that actually did the damage was a bare `python3 e2e/mutants.py restore` typed as a diagnostic during an investigation, so a lock guarding only the battery script would not have stopped it. The dangerous operation was the one that did not look like a write. Etiquette (`announce before touching`) had already failed three times because neither party can see the other's writes; this converts an invisible mutual-corruption race into an immediate legible refusal."
  affects: ["app/e2e/mutants.py", "app/e2e/mutation-battery.sh", 1.13.2, "any later node that mutates app/src"]
  check: "cd app && L=\"${TMPDIR:-/tmp}/find-e2e-mutation.lock\"; mkdir -p \"$L\"; echo $$ > \"$L/owner\"; python3 e2e/mutants.py restore; echo $?; rm -rf \"$L\"   # prints REFUSING and 1"
  closes: []
  status: verified

- id: A103
  phase: 5
  node: 1.13.2
  route: verifiable
  claim: "The `G2 SURVIVED` in the top-level session's battery run is VOID — an artifact of this node's `mutants.py restore` reverting G2's mutant mid-suite, not a real untested guard. Determined by re-running twice under the lock at ea99881b: G2 is killed, by 2 specs, both times. The mechanism is exact and was predicted before the run: `restore: () => {}` still lets `request` fire with the correct answers, so the question text renders and the funnel LOOKS right, but state.answers stays [] so canGoBack is false and the Back control never renders. The two killing specs are precisely the two that depend on that — 'a one-answer URL restores that answer...' (asserts Back visible) and 'the in-page Back rewrites... at a shared link's own depth' (clicks Back). This node's earlier reconstruction blaming G1 was also wrong: the consumed backup was stamped 13:51:24, after the empty-effect state observed at 13:49, i.e. the battery had already finished G1, restored it, and applied G2."
  because: "possibility (b) — that G2 genuinely survives and is a real hole — had to be excluded by measurement rather than by preferring the tidier story, since A74's whole finding is that a guard which does not die must be reported rather than explained away, and this run has repeatedly turned out to be that shape. It is not: G2 is not an equivalent mutant, it is observably killable, and the kill is mechanism-explicable rather than incidental. Recorded because anyone later reading that run's log will see `G2 SURVIVED` with no indication it was cross-run interference."
  affects: [1.13.2, "app/e2e/mutation-battery.sh", "the top-level session's battery log"]
  check: "cd app && bash e2e/mutation-battery.sh   # G2 killed by 2 failed; survivors [G18]"
  closes: []
  status: verified
