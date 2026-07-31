-- Staker digest ("since you were last here") + the streak counters the
-- digest's third item kind reads. See
-- docs/plans/2026-08-01-staker-digest-design.md and
-- indexer/src/handlers/digest.rs.
--
-- `lastXpDate` (006_xp_levels.sql) is the streak's anchor date — there is
-- deliberately no fourth "streakLastDate" column; the streak is advanced in
-- the same UPDATE that writes `lastXpDate` (indexer/src/handlers/xp.rs's
-- `award`), so the two can never disagree.

ALTER TABLE "User" ADD COLUMN IF NOT EXISTS "digestSeenAt" TIMESTAMP(3);
ALTER TABLE "User" ADD COLUMN IF NOT EXISTS "streakDays" INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "User" ADD COLUMN IF NOT EXISTS "streakBestDays" INTEGER NOT NULL DEFAULT 0;

-- Watermark starts at MIGRATION TIME, not "createdAt": rank-move items are
-- watermark-filtered, so backfilling to account-creation would make every
-- existing user's first panel a wall of accumulated moves from the beginning
-- of time. Starting at now() means the first panel shows only what changed
-- after this ships. `WHERE ... IS NULL` keeps a replay from resetting a
-- watermark a user has since advanced themselves.
UPDATE "User" SET "digestSeenAt" = now() WHERE "digestSeenAt" IS NULL;

-- Backfill both streak columns from the daily_bonus history that already
-- exists, so nobody's streak restarts at 0 the day this ships. Same
-- "derive it from the events we already recorded" precedent as
-- xp.rs::backfill.
--
-- 007_xp_daily_cap.sql's ("userId", kind, "awardDate") unique index
-- guarantees at most one daily_bonus row per user per UTC day, so the
-- classic gaps-and-islands trick applies directly: consecutive dates minus
-- their row number are constant within a run, so that difference IS the run
-- identity.
--
--   current = the run ending today or yesterday (a run that ended earlier is
--             already broken -> 0). At most one run can qualify, since two
--             runs ending that close together would be one contiguous run.
--   best    = the longest run the user has ever had.
WITH runs AS (
    SELECT
        "userId",
        "awardDate",
        "awardDate" - (ROW_NUMBER() OVER (PARTITION BY "userId" ORDER BY "awardDate"))::int AS run_id
    FROM "XpEvent"
    WHERE kind = 'daily_bonus'
), lengths AS (
    SELECT "userId", COUNT(*)::int AS len, MAX("awardDate") AS ended_on
    FROM runs
    GROUP BY "userId", run_id
), totals AS (
    SELECT
        "userId",
        COALESCE(MAX(len) FILTER (WHERE ended_on >= (now() AT TIME ZONE 'UTC')::date - 1), 0) AS current_len,
        MAX(len) AS best_len
    FROM lengths
    GROUP BY "userId"
)
UPDATE "User" u
SET "streakDays" = totals.current_len,
    "streakBestDays" = GREATEST(u."streakBestDays", totals.best_len)
FROM totals
WHERE u.id = totals."userId";
