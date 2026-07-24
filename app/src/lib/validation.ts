import { z } from "zod";

// Shared request validation schemas.

export const voteSchema = z.object({
  appId: z.string().min(1),
  amount: z.number().positive().max(1_000_000_000),
  txSig: z.string().min(32).max(128).optional(),
});

export const stakeSchema = z.object({
  appTagId: z.string().min(1),
  amount: z.number().positive().max(1_000_000_000),
  txSig: z.string().min(32).max(128).optional(),
});

export const unstakeSchema = z.object({
  stakeId: z.string().min(1),
});

export const unvoteSchema = z.object({
  voteId: z.string().min(1),
});

// Withdraws `amount` (up to the full active total, possibly spanning more
// than one active row) off a (user, target)'s aggregated position — see
// indexer/src/handlers/votes.rs and stakes.rs's withdraw_partial doc
// comments. Used by the profile page's "Your stakes" unstake action, which
// withdraws this same amount on-chain.
export const unstakePartialSchema = z.object({
  appTagId: z.string().min(1),
  amount: z.number().positive().max(1_000_000_000),
});

export const unvotePartialSchema = z.object({
  appId: z.string().min(1),
  amount: z.number().positive().max(1_000_000_000),
});

export const trackViewSchema = z.object({
  appId: z.string().min(1),
  path: z.string().max(300).optional().default("/"),
  referrer: z.string().max(300).optional(),
  turnstileToken: z.string().nullable().optional(),
});

// Requests to the indexer-tx-building proxy routes (see
// app/src/app/api/tx/**, app/src/lib/indexerClient.ts). `amount` is a raw
// on-chain u64 (already scaled by the token's decimals — see
// lib/anchorClient.ts's toRawAmount), passed as a decimal string since it
// can exceed JS's safe integer range.
const pubkeyString = z.string().min(32).max(44);
const rawAmountString = z.string().regex(/^[0-9]+$/, "must be a raw integer amount");

// App/tag ids are on-chain PDA seeds now (MAX_APP_ID_LEN/MAX_TAG_ID_LEN in
// programs/nebulous_world/src/constants.rs), capped at 32 bytes.
const seedIdString = z.string().min(1).max(32);

export const buildCreateAppTxSchema = z.object({
  appId: seedIdString,
  url: z.string().url().max(300),
  user: pubkeyString,
  tags: z.array(seedIdString).max(10).optional().default([]),
  // Metadata with no on-chain AppAccount field (see state/app.rs) — carried
  // through as a memo instruction by indexer/src/api.rs's build_create_app,
  // not written to Postgres directly by this app. All optional.
  name: z.string().max(80).optional(),
  tagline: z.string().max(140).optional(),
  description: z.string().max(4000).optional(),
  iconUrl: z.string().url().max(300).optional().or(z.literal("")),
  category: z.string().max(40).optional(),
  chain: z.string().max(40).optional(),
});

export const buildSuggestTagTxSchema = z.object({
  appId: seedIdString,
  tagSlug: seedIdString,
  user: pubkeyString,
});

export const buildVoteTxSchema = z.object({
  appId: z.string().min(1),
  amount: rawAmountString,
  user: pubkeyString,
});

export const buildStakeTagTxSchema = z.object({
  appId: z.string().min(1),
  tagSlug: z.string().min(1),
  amount: rawAmountString,
  user: pubkeyString,
});

export const buildClaimVoteRewardTxSchema = z.object({
  appId: z.string().min(1),
  user: pubkeyString,
});

export const buildClaimTagRewardTxSchema = z.object({
  appId: z.string().min(1),
  tagSlug: z.string().min(1),
  user: pubkeyString,
});

const claimItemSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("vote"), appId: z.string().min(1) }),
  z.object({ kind: z.literal("tag"), appId: z.string().min(1), tagSlug: z.string().min(1) }),
]);

export const buildClaimAllRewardsTxSchema = z.object({
  claims: z.array(claimItemSchema).min(1),
  user: pubkeyString,
});

// Closing a zero-stake VotePosition/StakePosition — the position's own
// pubkey is enough (see indexer/src/api.rs's build_close_vote_position doc
// comment): the on-chain instruction re-derives its seeds from the
// position's own stored `app`/`app_tag_stake` field, so no appId/tagSlug is
// needed here the way the other tx-building schemas above require.
export const buildClosePositionTxSchema = z.object({
  position: pubkeyString,
  user: pubkeyString,
});

export const buildBuyNebTxSchema = z.object({
  usdcAmount: z.number().positive().max(1_000_000_000),
  user: pubkeyString,
});

export const submitTxSchema = z.object({
  signedTransaction: z.string().min(1),
});

export const authVerifySchema = z.object({
  wallet: z.string().min(32).max(64),
  signature: z.string().min(32).max(200),
  nonce: z.string().min(8),
  message: z.string().min(8).max(1000),
});

// Filters are either onchain (tags, and the token stake behind them) or
// offchain (OpenGraph-derived text: name/tagline/description) — there is no
// separate "category" taxonomy on the search API.
const intFilter = () => z.coerce.number().int().min(0).optional();

export const searchSchema = z.object({
  q: z.string().max(120).optional().default(""),
  tags: z.array(z.string()).optional().default([]),
  fuzzy: z.string().max(120).optional().default(""),
  appStakeMin: intFilter(),
  appStakeMax: intFilter(),
  tagsStakeMin: intFilter(),
  tagsStakeMax: intFilter(),
  tagsCountMin: intFilter(),
  tagsCountMax: intFilter(),
  pageviewsMin: intFilter(),
  pageviewsMax: intFilter(),
  sort: z
    .enum(["rank", "votes", "stake", "traffic", "new", "trending_week", "trending_month"])
    .optional()
    .default("rank"),
  page: z.coerce.number().int().min(1).optional().default(1),
  pageSize: z.coerce.number().int().min(1).max(50).optional().default(20),
});
export type SearchInput = z.infer<typeof searchSchema>;
