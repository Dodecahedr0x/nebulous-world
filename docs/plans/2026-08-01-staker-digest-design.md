# Staker digest — "since you were last here"

## Problem

Nothing pulls a wallet user back. A staker's state genuinely changes between
visits — epochs settle into claimable rewards, staked apps move in rank, the
daily XP bonus resets — but none of that signal reaches anyone. There is no
watchlist, notification, subscription, or email primitive anywhere in the
codebase.

## Scope

A navbar bell, visible only to signed-in wallet users, opening a panel of what
changed since their last visit. Three item kinds: **claimable rewards**, **rank
moves on staked apps**, **streak status**.

Deliberately **on-site only**. No web push, no email, no PII. This deepens a
visit rather than causing one; the payload is computed in one place so a
delivery channel can consume it later without recomputation.

Explicitly out of scope: per-item read state, activity-by-others items,
anonymous/no-wallet digests.

## Architecture

The indexer owns the database, so all computation lives there and the app
proxies. The browser never touches the indexer.

```
Navbar → DigestBell (client)
       → useDigest() → /api/digest        (Next route, reads session cookie)
                     → indexerClient      (server-only)
                     → GET /digest?userId (indexer, Rust/sqlx)
```

### Migration `010_digest_and_streak.sql`

Three columns on `User`:

| Column | Type | Purpose |
|---|---|---|
| `digestSeenAt` | `TIMESTAMP(3)` | Watermark. Backfilled to **migration time**, not `createdAt` — otherwise every existing user's first panel is a wall of accumulated rank moves. |
| `streakDays` | `INTEGER NOT NULL DEFAULT 0` | Current consecutive-day run. |
| `streakBestDays` | `INTEGER NOT NULL DEFAULT 0` | Personal best, so a broken streak leaves something behind. |

`lastXpDate` already exists and is the streak's anchor date — no fourth column.

Backfill both streak columns from existing `XpEvent` rows of kind
`daily_bonus`: current streak = the run of consecutive `awardDate`s ending at
today or yesterday (a run ending earlier is already broken → 0); best = the
longest such run overall. Follows the precedent set by `xp::backfill`.

### Endpoints

- `GET /digest?userId=…` → the DTO below
- `POST /digest/seen` (body `{ userId }`) → sets `digestSeenAt = now()`,
  returns the new watermark

New handler `indexer/src/handlers/digest.rs`, merged into `api.rs::router`
alongside the others.

## DTO contract

This is the interface both workstreams build against.

```jsonc
// GET /digest?userId=<id>
{
  "seenAt": "2026-07-30T12:00:00Z", // null if never opened
  "count": 3,                        // badge number, see below
  "items": [
    { "kind": "reward",    "appId": "…", "appSlug": "jupiter", "appName": "Jupiter",
      "appIconUrl": "…", "amount": 12.5, "epochCount": 2 },
    { "kind": "rank_move", "appId": "…", "appSlug": "…", "appName": "…",
      "appIconUrl": "…", "from": 7, "to": 4, "delta": 3 },
    { "kind": "streak",    "streakDays": 5, "bestDays": 9,
      "bonusClaimedToday": false }
  ]
}
```

Item order in the array is the render order: rewards, then rank moves, then
streak.

`amount` is a **JSON number in whole-token units**, rendered with
`formatToken` and *not* scaled by `voteTokenDecimals`. `RevenueClaim.amount`
is a `DOUBLE PRECISION` accounting column, the same family as `Stake.amount`
(cf. `MyStakes`' `stakedAmount`, rendered unscaled). The
decimal-string-for-u64/u128 rule in `indexerClient.ts`'s header governs a
different path entirely: raw amounts read off an on-chain account, like
`MyStakes`' `position.amount`, which *does* get scaled via `fromRawAmount`.

## Item semantics

### Claimable rewards — outstanding, not watermark-filtered

Money is a *state*, not an event. If you saw "12 NEB claimable" yesterday and
didn't claim, it must still be there today; watermark-filtering would silently
hide unclaimed money the moment you opened the panel once.

**Corrected during implementation.** The obvious formulation — distributed
epochs the user has *no* `RevenueClaim` for — is wrong.
`revenue.rs::settle_epoch` inserts exactly one `RevenueClaim` per participant
*at settle time*, so that predicate excludes every epoch the user is actually
owed from (permanently empty), and `RevenueEpoch.grossRevenue` is the whole
app's revenue, not this user's share. Key off the user's **unclaimed
`RevenueClaim`** instead — right set, right per-user amount, same
"already-claimed epochs excluded" semantics:

```sql
SELECT a.id, a.slug, a.name, a."iconUrl",
       SUM(c.amount)::double precision AS amount, COUNT(*) AS epoch_count
FROM "RevenueClaim" c
JOIN "RevenueEpoch" e ON e.id = c."epochId"
JOIN "App" a ON a.id = e."appId"
WHERE c."userId" = $1 AND c.claimed = false AND e.distributed = true
  AND EXISTS (SELECT 1 FROM "Stake" s JOIN "AppTag" at ON at.id = s."appTagId"
              WHERE at."appId" = e."appId" AND s."userId" = $1 AND s.active)
GROUP BY a.id, a.slug, a.name, a."iconUrl"
```

One row per app, amounts summed, `epochCount` = epochs rolled into it. Links
to `/rewards`.

### Rank moves — watermark-filtered

For each app in the user's active stake set, compare today's rank against the
newest `AppStatsSnapshot` at or before `digestSeenAt`. **Position**, not raw
`rankScore` — position is the legible unit. Computed per snapshot date with
`RANK() OVER (PARTITION BY "date" ORDER BY "rankScore" DESC)`; at current
catalog size that is a trivial window over a few hundred rows.

Two noise guards: suppress moves smaller than **±2 positions**, and cap at the
**top 3 movers by absolute delta**.

Down-moves are shown, not just up. Hiding them would violate Product Principle
4, and a staker who cannot see a position slipping cannot act on it. Per
Accessibility, direction must never be conveyed by colour alone — pair with an
arrow glyph and a signed number.

If `digestSeenAt` is null, or no snapshot exists at or before it, emit no
rank-move items rather than comparing against the beginning of time.

### Streak

Mutation belongs in `xp::award()`, **not** in the digest — that function
already knows "this user did a qualifying thing today" and already owns the
once-per-UTC-day atomicity boundary. Inside the existing
`if last_xp_date != Some(today)` block, in the same `UPDATE` that writes
`lastXpDate`:

- `last_xp_date == Some(today - 1)` → `streakDays += 1`
- otherwise → `streakDays = 1`
- always → `streakBestDays = GREATEST("streakBestDays", "streakDays")`

Note the daily bonus is **auto-awarded on any qualifying action**, not claimed
by showing up. The panel copy must say so honestly: "vote, stake, or suggest a
tag to keep your streak" — never "claim your bonus", which implies a button
that does not exist.

`bonusClaimedToday` is derived from `lastXpDate == today`.

## Badge count

`count` = reward rows + rank-move rows + the streak row **only when
`bonusClaimedToday` is false**.

The streak row is always *shown* in the panel when the user has any streak
state, but only *counted* when today's bonus is still unearned — i.e. when the
streak is actually at risk. This keeps the badge strictly actionable instead of
displaying a permanent "1".

A number, not a dot: each row is a distinct actionable thing, so the number is
honest, and "3" earns a click in a way a dot does not.

## UI

`DigestBell` client component in `Navbar.tsx`, rendered only when `connected`,
next to the wallet button and the existing level badge. Backed by a
`useDigest()` hook following the `useUserLevel` shape — `useLiveQuery`, keyed
on `user`, `enabled: !!user`.

Polls on mount and route change rather than on a short timer; a returning visit
is the trigger, not elapsed seconds.

Opening the panel fires `POST /api/digest/seen`, advancing the watermark. The
panel's currently rendered contents stay put for that view — the watermark
advance affects the *next* load, so nothing vanishes under the user's cursor.
Rewards, being state rather than event, survive the advance regardless.

Empty state: bell renders with no badge; panel says everything is up to date.
Signed out: no bell at all.

## Simulation mode

Every flow must work with no funded wallet (Principle 3). Rewards items simply
come back empty when no epoch has settled; rank moves and streak are entirely
off-chain-derived and work identically in both modes. No branch needed.

## Testing

- Rust unit tests for streak transitions: first ever day, consecutive day, gap
  of one day, gap of many, best-day ratchet, same-day double-award idempotence.
- Rust tests for badge count semantics, especially the streak-row-not-counted
  case.
- Rank-move tests: null watermark, no snapshot before watermark, sub-threshold
  move suppressed, more than three movers capped, down-move present.
- Rewards test: an already-claimed epoch is excluded; an unclaimed one survives
  a watermark advance.
