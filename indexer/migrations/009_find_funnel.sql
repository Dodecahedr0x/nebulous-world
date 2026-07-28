-- The `/find` funnel's two persisted tables. In-flight funnel state is NOT
-- stored: the client posts its whole answer history on every request (A1), so
-- the only thing worth a row is a session that actually finished. See
-- indexer/src/find/store.rs, the sole reader and writer of both tables.
--
-- `visitorId`/`sessionId` are the existing anonymous tracking identities (A2) —
-- a salted HMAC of IP+UA computed in app/src/lib/tracking.ts. Never PII, never
-- a wallet: /find works with nothing connected, so there is no other identity
-- available and deliberately none required.

-- The unique index below is what makes POST /find/confirm idempotent per
-- ("sessionId", "appId", "outcome") (A19): a replay inserts nothing and the
-- handler reports it as a no-op. That is a correctness property of the
-- anti-farming design rather than a tidiness measure — an outcome that could
-- be re-submitted N times is exactly the thing brief criterion 4 forbids — so
-- it belongs in the schema, where no code path can forget it, instead of in an
-- `if` in Rust that a second call site would have to remember to repeat.
--
-- `answers` is JSONB rather than a child table because the answer path is only
-- ever read whole (store::load_learned never looks inside it) and because
-- keeping the write to a single statement is what lets the unique index above
-- do the idempotency work atomically.
--
-- `turnstileVerified` records whether the confirm that produced this row
-- cleared Turnstile, instead of the route refusing the write (A80 supersedes
-- A29). A refused write is not a neutral loss: Turnstile fails hardest for VPN
-- users, privacy browsers and slow connections, so dropping those rows biases
-- the training set toward one kind of visitor and makes the drop rate
-- unmeasurable at the same time. Recording the row and training only on the
-- verified ones buys farming nothing — store::load_learned filters on this
-- column — while leaving the loss countable.
--
-- DEFAULT true, and the default is only ever reached by a writer that omits
-- the column. The two cases that column distinguishes are not symmetric:
-- "Turnstile was configured and the token failed" always arrives through
-- store::record_outcome, which binds the flag explicitly and so never falls
-- through to the default; the only writer that can omit it is one with no
-- notion of Turnstile at all, i.e. local and simulation mode, where nothing is
-- configured and there is nothing to verify. Defaulting those rows to false
-- would make load_learned return an empty map for every local run — the
-- learning loop dead exactly where A4 requires the funnel to work with no
-- infrastructure — to defend against a path that cannot use the default.
--
-- No foreign key to "App", unlike "PageView", which cascades from it. An
-- outcome is a record of what a visitor was shown and what they said about it;
-- if the app row later disappears, the write must still not fail and the
-- historical row must still not vanish. The learned term simply stops seeing
-- an app id that no longer joins to anything, which is the correct behaviour
-- for a signal that only ever ranks apps that currently exist.
CREATE TABLE IF NOT EXISTS "FindSession" (
    "id" TEXT NOT NULL,
    "visitorId" TEXT NOT NULL,
    "sessionId" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "outcome" TEXT NOT NULL,
    "answers" JSONB NOT NULL DEFAULT '[]'::jsonb,
    "questionsAsked" INTEGER NOT NULL DEFAULT 0,
    "turnstileVerified" BOOLEAN NOT NULL DEFAULT true,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "FindSession_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX IF NOT EXISTS "FindSession_sessionId_appId_outcome_key"
    ON "FindSession" ("sessionId", "appId", "outcome");

-- store::load_learned aggregates by "appId"; "createdAt" carries the eventual
-- "learn from recent sessions only" window and criterion 7's reporting slice.
CREATE INDEX IF NOT EXISTS "FindSession_appId_idx" ON "FindSession" ("appId");
CREATE INDEX IF NOT EXISTS "FindSession_createdAt_idx" ON "FindSession" ("createdAt");

-- The question-ordering half of the learning loop: which facets people actually
-- answer, as opposed to "FindSession", which records which app they picked.
-- A facet that is overwhelmingly skipped is one users cannot answer, and that
-- is a fact about the question, not about the catalog — so it is counted
-- separately from any app.
CREATE TABLE IF NOT EXISTS "FindFacetStat" (
    "facetKind" TEXT NOT NULL,
    "facetValue" TEXT NOT NULL,
    "yesCount" INTEGER NOT NULL DEFAULT 0,
    "noCount" INTEGER NOT NULL DEFAULT 0,
    "skipCount" INTEGER NOT NULL DEFAULT 0,
    "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "FindFacetStat_pkey" PRIMARY KEY ("facetKind", "facetValue")
);
