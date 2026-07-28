# Brief — `/find`, a guided app-discovery funnel

## What we're building

A new top-level surface at `/find` on nebulous.world that suggests the single
best-matching app for a visitor who does **not** know what they're looking for.
It asks a short series of questions, each chosen to maximally discriminate
among the remaining candidates, and converges on a ranked shortlist headed by
one suggestion.

Akinator-shaped in mechanic, but the goal is *recommendation*, not
guess-the-thing-you're-already-thinking-of. The user has a need, not an answer.

Two scoring inputs, blended:

1. **App data** (primary) — the facets the catalog already carries: `category`
   (16 fixed values), `chain`, crowd-sourced tags via `AppTag`, plus
   `stakeTotal` / `viewCount` / `rankScore` bands.
2. **Prior user answers** (secondary, capped) — completed sessions train two
   things: which apps satisfied which answer paths (outcome weights), and which
   facets actually discriminate between users (question ordering).

Answers **score** rather than filter: every answer adjusts a per-app score, so
the shortlist is never empty. This matters specifically because tags are
crowd-sourced — a missing tag is not evidence of absence.

**Engine mechanics, as corrected by Phase 2 research** (`research/`):

- Question choice maximizes **expected information gain** (mutual information)
  over the normalized candidate distribution. Explicitly *not* the "splits the
  set closest to 50/50" rule — that is exactly equivalent to max IG only when
  answers are deterministic, and generalized binary search degrades badly under
  the soft/noisy answers we deliberately chose (`question-selection.md`, A11).
- Attributes are **three-valued**: present / known-absent / never-recorded.
  Never-recorded marginalizes over a tag-prevalence prior rather than counting
  as negative evidence — this is the specific mechanism that stops well-tagged
  apps from dominating thin ones (`soft-scoring-update.md`, A14).
- Likelihoods accumulate in **log space, clamped to `[ε, 1−ε]`**, so no
  candidate is ever eliminated and one wrong answer costs a bounded
  `log((1−ε)/ε)` rather than being unrecoverable.
- Answer vocabulary is exactly **Yes / No / Don't care**. "Don't care" performs
  no update at all. Hedged answers are out of scope — no citable likelihood
  exists for them (A15).
- Stopping is a **posterior-probability threshold** (`p_top1 ≥ 1−δ`) or a
  normalized-entropy floor — not a top1−top2 margin, which has no optimality
  result behind it (A12). The user-facing behaviour is unchanged: stop when
  confident, hard cap 8, always escapable.
- Known modelling risk: conditional independence across answers is **false**
  for our facets (a "lending" tag implies category "defi"), so correlated
  evidence over-sharpens the posterior — which matters precisely because the
  posterior drives the stopping rule (A13). Must be addressed, not assumed away.

Question selection, scoring, session storage, and the learning loop all live in
the **indexer** (Rust/`sqlx`). The browser holds only the current question, the
user's own answer history, and the final shortlist.

The funnel stops when the top candidate separates from the pack by a threshold,
hard-caps at 8 questions, and exposes "show me results", back, and start-over
from the first question onward. It ends on an explicit **"Is this the one?"**
confirm — Akinator's own move, and the strong training signal.

## Constraints

**Architectural (from `AGENTS.md`, non-negotiable):**

- `app/` has no database client and no Solana RPC access. Every read/write goes
  through the indexer's HTTP API via `app/src/lib/indexerClient.ts`. New
  server-side logic is a Rust handler in `indexer/src/handlers/`, fronted by a
  Next.js route in `app/src/app/api/**/route.ts`.
- All DDL is a new numbered migration in `indexer/migrations/` (next: `009_`),
  applied at indexer startup by `sqlx::migrate!()`. `app/prisma/schema.prisma`
  is codegen-input only and must be hand-synced if touched.
- Pure logic stays separate from DB/IO so the scoring math is unit-testable
  without a database — mirrors `ranking.ts` / `reward_math.rs`.
- Repo style: `handler()` + `ok()`/`fail()` + Zod validation on app routes;
  `Result<_, ApiError>` in indexer handlers; named function declarations in
  components; Tailwind via the shared `.card`/`.btn`/`.chip` primitives.

**Product (from `PRODUCT.md`):**

- Must work fully in **simulation mode** with no funded wallet. `/find` is
  read-only; using it never requires a wallet connection.
- Voting/staking must remain reachable on the result cards, not gated behind a
  detail page (Principle 1) — reuse the existing `AppCard`.
- Rankings must stay legible as *why*, not just *what* (Principle 2) — the
  result should be able to say what the answers actually did.
- Never imply traction that isn't real (Principle 4) — no "N people found this"
  copy until the sessions genuinely exist.
- WCAG 2.1 AA; color never the sole signal.

**Commercial — the catalog is a monetized asset.**
`/api/data/*` sells catalog/ranking data per-request in NEB via x402. The
`/find` endpoints must therefore never return the candidate set with its
facets, and never expose a bulk snapshot. Each response carries the next
question plus a bounded shortlist — nothing that reconstructs the database.
This is *why* the engine is server-side, and it is a stricter requirement than
"don't ship a big payload".

**Anti-gaming — two mechanisms, not one.** Surfacing in `/find` drives traffic,
which drives ad revenue, which accrues to stakers, so farming confirmations is
directly profitable. Phase 2 research (`cold-start-blending.md`, A16) corrected
an important error here: confidence shrinkage does **not** bound an adversary,
because the support count `v` is attacker-controlled and `v/(v+m) → 1`. The
Wilson lower bound fails for the same reason. So:

1. **Shrinkage** — `(v/(v+m))·R + (m/(v+m))·C` *inside* the learned term, with
   `C` a neutral prior. Handles cold start and small-sample noise only.
2. **A separate hard cap** — `final = content + λ·learned`, `learned ∈ [0,1]`,
   with `λ` strictly below the minimum content-score gap. This is what makes
   farming unprofitable at any volume. It has a named ancestor (Resnick & Sami's
   influence limiter, RecSys 2007), but no retrieved source measures its effect
   in a content+CF hybrid — so the no-flip property is **algebra we assert and
   unit-test**, not a borrowed guarantee. That is success criterion 4.

Cloudflare Turnstile (`lib/turnstile.ts`) and the existing fixed-window rate
limiter (`lib/rateLimit.ts`) gate the confirm endpoint. Describe this in code
and copy as a "bounded-influence blend plus Turnstile and rate limiting" — the
shilling literature actually recommends detection and robust training, not
static caps, and overstating the pedigree would mislead the next reader (A18).

**Cold start.** There are zero logged sessions today and ~135 catalog apps. The
app-data score must carry v1 entirely on its own; the learned term blends in
only as support accumulates, weighted by its own confidence. Burke's warning
about weighted hybrids applies directly — they assume component value is
roughly uniform across items, which is false at this catalog size and session
count, and is a second independent argument for a small `λ`.

## Success criteria

1. A visitor with no wallet and no crypto fluency reaches a relevant app in
   under 8 questions, and can always bail to results early.
2. The shortlist is never empty, for any answer path, including all-"don't
   care" and self-contradictory paths.
3. No `/find` response can be replayed to reconstruct the catalog — verified by
   inspecting the actual response bodies, not by intent.
4. Farming N confirmations on a deliberately bad match cannot move it above a
   well-matched app, for any N — demonstrable as a test.
5. Scoring and question-selection math are unit-tested with no database.
6. Works identically in simulation mode and on-chain mode.
7. Average questions-to-confirm is measurable, so the self-tuning loop's effect
   is observable rather than assumed.
