// Server-only client for the indexer's HTTP API (indexer/src/api.rs) — the
// app's ONE gateway to on-chain state and transaction submission. Nothing
// in this codebase should construct a Solana `Connection` or talk to an
// RPC endpoint directly anymore; every read/build/submit goes through here
// instead, proxied to the actual indexer service (which does own an RPC
// connection — see indexer/README.md's architecture note). Only ever
// imported from `app/src/app/api/**` route handlers (server-side) — never
// from a "use client" component, since the indexer isn't reachable from
// the browser (see render.yaml: it's a private, internal-network-only
// service, same as it always was).
//
// u64/u128 on-chain values are passed through as decimal STRINGS end to
// end (indexer -> here -> the Next.js API route -> the browser), never as
// JS numbers or a BN class instance — BN has no JSON.stringify support
// (it would serialize its internal {negative, words, length} shape, not a
// number), and plain numbers lose precision above 2^53. Whichever piece
// of client code needs to actually do arithmetic on one of these (e.g.
// ClaimRewards.tsx's settlePendingRaw) wraps it in `new BN(str)` itself,
// right before use.

import { config } from "@/lib/config";
import type { AppDTO, AppDetail, SearchResult } from "@/lib/types";
import type { SearchInput } from "@/lib/validation";

const INDEXER_API_URL = config.indexerApiUrl;

class IndexerNotFoundError extends Error {}

/** Every indexer error response body is `{"error": "message"}"` — see indexer/src/api.rs's `ApiError`. */
async function errorMessage(res: Response, fallback: string): Promise<string> {
  const text = await res.text().catch(() => "");
  try {
    const parsed = JSON.parse(text) as { error?: string };
    return parsed.error ?? fallback;
  } catch {
    return text || fallback;
  }
}

async function request(method: "GET" | "POST" | "PATCH", path: string, body?: unknown): Promise<unknown> {
  const res = await fetch(`${INDEXER_API_URL}${path}`, {
    method,
    headers: body !== undefined ? { "Content-Type": "application/json" } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
    cache: "no-store",
  });
  if (method === "GET" && res.status === 404) throw new IndexerNotFoundError(path);
  if (!res.ok) {
    throw new Error(await errorMessage(res, `indexer ${method} ${path} failed (${res.status})`));
  }
  return res.json();
}

const get = (path: string) => request("GET", path);
const post = (path: string, body: unknown) => request("POST", path, body);
const patch = (path: string, body: unknown) => request("PATCH", path, body);

/** `null` on a 404 (the account genuinely doesn't exist yet), throws on any other failure. */
async function getOrNull(path: string): Promise<unknown | null> {
  try {
    return await get(path);
  } catch (err) {
    if (err instanceof IndexerNotFoundError) return null;
    throw err;
  }
}

export interface AppAccountData {
  pda: string;
  appId: string;
  totalVoteStake: string;
  voteAccRewardPerShare: string;
  totalTagStake: string;
  tagsAccRewardPerShare: string;
  bump: number;
}

export interface AppTagStakeData {
  pda: string;
  app: string;
  tag: string;
  tagId: string;
  stakeAmount: string;
  bump: number;
}

export interface PositionData {
  pda: string;
  owner: string;
  /** Who paid this position's rent at creation — see indexer/src/api.rs's `PositionRow::payer` doc comment. */
  payer: string;
  amount: string;
  rewardDebt: string;
  /** Unix seconds — see app/src/lib/unstakeFee.ts for what this drives. */
  stakedAt: number;
  bump: number;
}

export async function fetchAppAccount(appId: string): Promise<AppAccountData | null> {
  return (await getOrNull(`/accounts/app/${encodeURIComponent(appId)}`)) as AppAccountData | null;
}

export async function fetchAppTagStake(
  appId: string,
  tagSlug: string,
): Promise<AppTagStakeData | null> {
  return (await getOrNull(
    `/accounts/app-tag/${encodeURIComponent(appId)}/${encodeURIComponent(tagSlug)}`,
  )) as AppTagStakeData | null;
}

export async function fetchVotePosition(
  appId: string,
  owner: string,
): Promise<PositionData | null> {
  return (await getOrNull(
    `/accounts/vote-position/${encodeURIComponent(appId)}/${owner}`,
  )) as PositionData | null;
}

export async function fetchStakePosition(
  appId: string,
  tagSlug: string,
  owner: string,
): Promise<PositionData | null> {
  return (await getOrNull(
    `/accounts/stake-position/${encodeURIComponent(appId)}/${encodeURIComponent(tagSlug)}/${owner}`,
  )) as PositionData | null;
}

export interface WalletBalance {
  amount: string;
  decimals: number;
  uiAmountString: string;
}

export async function fetchBalance(owner: string, mint: string): Promise<WalletBalance> {
  return (await get(`/balances/${owner}/${mint}`)) as WalletBalance;
}

export interface NebPoolStatus {
  poolAddress: string;
  price: number;
  nebReserve: number;
  usdcReserve: number;
  nebMint: string;
  usdcMint: string;
}

export async function fetchPoolStatus(): Promise<NebPoolStatus | null> {
  return (await getOrNull("/pool")) as NebPoolStatus | null;
}

export interface BuiltTx {
  transaction: string;
}

export interface CreateAppInput {
  appId: string;
  url: string;
  user: string;
  tags?: string[];
  name?: string;
  tagline?: string;
  description?: string;
  iconUrl?: string;
  category?: string;
  chain?: string;
}

/** Builds "create app (+ initial tags)" as one atomic transaction — see indexer/src/api.rs's build_create_app. */
export async function buildCreateAppTx(input: CreateAppInput): Promise<BuiltTx> {
  return (await post("/tx/create-app", input)) as BuiltTx;
}

/** Adds a tag to an app that already exists (unlike buildCreateAppTx's bundled initial tags). */
export async function buildSuggestTagTx(
  appId: string,
  tagSlug: string,
  user: string,
): Promise<BuiltTx> {
  return (await post("/tx/suggest-tag", { appId, tagSlug, user })) as BuiltTx;
}

/** `amount` is a raw, already-decimals-scaled u64 as a decimal string (see lib/anchorClient.ts's toRawAmount). */
export async function buildVoteTx(appId: string, amount: string, user: string): Promise<BuiltTx> {
  return (await post("/tx/vote", { appId, amount, user })) as BuiltTx;
}

export async function buildWithdrawVoteTx(
  appId: string,
  amount: string,
  user: string,
): Promise<BuiltTx> {
  return (await post("/tx/withdraw-vote", { appId, amount, user })) as BuiltTx;
}

export async function buildStakeTagTx(
  appId: string,
  tagSlug: string,
  amount: string,
  user: string,
): Promise<BuiltTx> {
  return (await post("/tx/stake-tag", { appId, tagSlug, amount, user })) as BuiltTx;
}

export async function buildWithdrawTagStakeTx(
  appId: string,
  tagSlug: string,
  amount: string,
  user: string,
): Promise<BuiltTx> {
  return (await post("/tx/withdraw-tag-stake", { appId, tagSlug, amount, user })) as BuiltTx;
}

export async function buildClaimVoteRewardTx(appId: string, user: string): Promise<BuiltTx> {
  return (await post("/tx/claim-vote-reward", { appId, user })) as BuiltTx;
}

export async function buildClaimTagRewardTx(
  appId: string,
  tagSlug: string,
  user: string,
): Promise<BuiltTx> {
  return (await post("/tx/claim-tag-reward", { appId, tagSlug, user })) as BuiltTx;
}

export type ClaimItem = { kind: "vote"; appId: string } | { kind: "tag"; appId: string; tagSlug: string };

export interface BuiltTxs {
  transactions: string[];
}

/** Builds the minimum number of transactions packing every claim in `claims` — see api.rs's build_claim_all_rewards. */
export async function buildClaimAllRewardsTx(claims: ClaimItem[], user: string): Promise<BuiltTxs> {
  return (await post("/tx/claim-all-rewards", { claims, user })) as BuiltTxs;
}

export async function buildCloseVotePositionTx(position: string, user: string): Promise<BuiltTx> {
  return (await post("/tx/close-vote-position", { position, user })) as BuiltTx;
}

export async function buildCloseTagStakePositionTx(position: string, user: string): Promise<BuiltTx> {
  return (await post("/tx/close-tag-stake-position", { position, user })) as BuiltTx;
}

export interface CloseablePosition {
  position: string;
  kind: "vote" | "tagStake";
  /** Rent lamports this position refunds once closed. */
  lamports: number;
}

/** Every zero-stake VotePosition/StakePosition `owner` can reclaim rent from — see indexer/src/api.rs's get_closeable_positions. */
export async function fetchCloseablePositions(owner: string): Promise<CloseablePosition[]> {
  const { positions } = (await get(`/wallet/${owner}/closeable-positions`)) as {
    positions: CloseablePosition[];
  };
  return positions;
}

export interface BuiltSwap extends BuiltTx {
  /** Expected NEB output at quote time, UI units. */
  nebOut: number;
}

export async function buildBuyNebTx(usdcAmount: number, user: string): Promise<BuiltSwap> {
  return (await post("/tx/buy-neb/build", { usdcAmount, user })) as BuiltSwap;
}

export interface SubmitResult {
  signature: string;
}

export async function submitSignedTx(signedTransaction: string): Promise<SubmitResult> {
  return (await post("/tx/submit", { signedTransaction })) as SubmitResult;
}

export interface PlatformMetricsPoint {
  capturedAt: string;
  appCount: number;
  tagCount: number;
  /** Raw on-chain u64 amounts, decimal strings — scale by voteTokenDecimals. */
  totalVoteStake: string;
  totalTagStake: string;
}

/** Ascending time series written by indexer/src/platform_metrics.rs — the on-chain-derived half of the Explore page's metric trend charts. */
export async function fetchPlatformMetricsHistory(): Promise<PlatformMetricsPoint[]> {
  return (await get("/metrics/platform-history")) as PlatformMetricsPoint[];
}

// ---------------------------------------------------------------------
// Product-data endpoints (indexer/src/handlers/**) — everything that used
// to be a direct Prisma query from this app now goes through here instead,
// same as the on-chain reads above. See root AGENTS.md.
// ---------------------------------------------------------------------

export interface IndexerUser {
  id: string;
  wallet: string;
  handle: string | null;
}

export async function connectUser(wallet: string): Promise<IndexerUser> {
  return (await post("/users/connect", { wallet })) as IndexerUser;
}

export async function fetchUserById(id: string): Promise<IndexerUser | null> {
  return (await getOrNull(`/users/${encodeURIComponent(id)}`)) as IndexerUser | null;
}

export async function searchApps(input: SearchInput): Promise<SearchResult> {
  return (await post("/apps/search", input)) as SearchResult;
}

export async function fetchAppBySlug(slug: string): Promise<AppDetail | null> {
  return (await getOrNull(`/apps/by-slug/${encodeURIComponent(slug)}`)) as AppDetail | null;
}

export async function fetchAppById(id: string): Promise<AppDTO | null> {
  return (await getOrNull(`/apps/by-id/${encodeURIComponent(id)}`)) as AppDTO | null;
}

export async function fetchRelatedApps(query: { slugs?: string[]; tagSlugs?: string[] }): Promise<{ apps: AppDTO[] }> {
  const sp = new URLSearchParams();
  if (query.slugs?.length) sp.set("slugs", query.slugs.join(","));
  if (query.tagSlugs?.length) sp.set("tagSlugs", query.tagSlugs.join(","));
  return (await get(`/apps/related?${sp.toString()}`)) as { apps: AppDTO[] };
}

/**
 * The maps' "advanced search" — min/max app stake, min/max tag count,
 * min/max pageviews. Each value is a raw query-param string (or omitted),
 * same convention as Discover's `RangeFilters` (`components/discover/
 * FilterPanel.tsx`) — kept as its own smaller type here rather than
 * importing that one, since this lib module shouldn't depend on a
 * component file, and the maps only expose 3 of Discover's 4 range pairs
 * (no "tags stake").
 */
export interface MapRangeFilters {
  appStakeMin?: string;
  appStakeMax?: string;
  tagsCountMin?: string;
  tagsCountMax?: string;
  pageviewsMin?: string;
  pageviewsMax?: string;
}

const MAP_RANGE_KEYS: (keyof MapRangeFilters)[] = [
  "appStakeMin",
  "appStakeMax",
  "tagsCountMin",
  "tagsCountMax",
  "pageviewsMin",
  "pageviewsMax",
];

/** Pulls the maps' range-filter params out of an incoming request's own search params — shared by /api/apps/graph and /api/tags/pack. */
export function mapRangeFiltersFromParams(sp: URLSearchParams): MapRangeFilters {
  const ranges: MapRangeFilters = {};
  for (const key of MAP_RANGE_KEYS) {
    const value = sp.get(key);
    if (value) ranges[key] = value;
  }
  return ranges;
}

function rangeSearchParams(ranges?: MapRangeFilters): URLSearchParams {
  const sp = new URLSearchParams();
  if (!ranges) return sp;
  for (const [key, value] of Object.entries(ranges)) {
    if (value) sp.set(key, value);
  }
  return sp;
}

export interface AppGraph {
  nodes: { id: string; name: string; stake: number; views: number; votes: number }[];
  edges: { source: string; target: string; shared: number; weighted: number }[];
}

export async function fetchAppGraph(tags: string[] = [], ranges?: MapRangeFilters): Promise<AppGraph> {
  const sp = rangeSearchParams(ranges);
  if (tags.length > 0) sp.set("tags", tags.join(","));
  const qs = sp.toString();
  return (await get(`/apps/graph${qs ? `?${qs}` : ""}`)) as AppGraph;
}

export interface TagGraph {
  nodes: { id: string; name: string; stake: number; appCount: number }[];
  edges: { source: string; target: string; weight: number; similarity: number }[];
}

export async function fetchTagGraph(): Promise<TagGraph> {
  return (await get("/tags/graph")) as TagGraph;
}

export interface TagPack {
  tags: { slug: string; name: string; appCount: number; stake: number }[];
  apps: { slug: string; name: string; stake: number; tagSlugs: string[] }[];
}

/** Every approved app's full tag list, for the Explore page's Group (circle-packing) tab — see app/src/lib/tagPack.ts. */
export async function fetchTagPack(ranges?: MapRangeFilters): Promise<TagPack> {
  const qs = rangeSearchParams(ranges).toString();
  return (await get(`/tags/pack${qs ? `?${qs}` : ""}`)) as TagPack;
}

export interface TagListEntry {
  id: string;
  slug: string;
  name: string;
  appCount: number;
  stakeTotal: number;
}

export async function fetchTags(q?: string): Promise<{ tags: TagListEntry[] }> {
  const sp = q ? `?q=${encodeURIComponent(q)}` : "";
  return (await get(`/tags${sp}`)) as { tags: TagListEntry[] };
}

/** Exact-slug lookup, `null` if the tag doesn't exist — for /tags/[slug]'s metadata/404. */
export async function fetchTagBySlug(slug: string): Promise<TagListEntry | null> {
  return (await getOrNull(`/tags/by-slug/${encodeURIComponent(slug)}`)) as TagListEntry | null;
}

export interface PlatformStats {
  totalApps: number;
  totalTags: number;
  totalVoteWeight: number;
  totalStake: number;
  totalViews: number;
  /** Raw on-chain u64 amount (vote-token decimals), decimal string — scale by voteTokenDecimals. */
  totalRevenueDistributed: string;
}

export async function fetchPlatformStats(): Promise<PlatformStats> {
  return (await get("/platform/stats")) as PlatformStats;
}

export async function fetchPlatformViewsTrend(): Promise<{ date: string; totalViews: number }[]> {
  return (await get("/platform/views-trend")) as { date: string; totalViews: number }[];
}

/** Ascending daily time series of on-chain `fund_app_rewards` amounts — see indexer/src/handlers/platform.rs's platform_revenue_trend. */
export async function fetchRevenueDistributedTrend(): Promise<{ date: string; amount: string }[]> {
  return (await get("/platform/revenue-trend")) as { date: string; amount: string }[];
}

export async function fetchVote(appId: string, userId: string): Promise<{ id: string; amount: number } | null> {
  const res = (await get(`/votes?appId=${encodeURIComponent(appId)}&userId=${encodeURIComponent(userId)}`)) as {
    vote: { id: string; amount: number } | null;
  };
  return res.vote;
}

export async function createVote(input: {
  appId: string;
  userId: string;
  amount: number;
  txSig: string | null;
}): Promise<{ vote: { id: string; amount: number; txSig: string | null }; app: { voteWeight: number; voteCount: number; rankScore: number } }> {
  return (await post("/votes", input)) as {
    vote: { id: string; amount: number; txSig: string | null };
    app: { voteWeight: number; voteCount: number; rankScore: number };
  };
}

export async function withdrawVote(voteId: string, userId: string): Promise<{ withdrawn: boolean }> {
  return (await post(`/votes/${encodeURIComponent(voteId)}/withdraw`, { userId })) as { withdrawn: boolean };
}

/** Withdraws `amount` off this (user, app)'s active Vote rows, oldest-first — see indexer/src/handlers/votes.rs's withdraw_partial. */
export async function withdrawPartialVotes(
  appId: string,
  amount: number,
  userId: string,
): Promise<{ withdrawn: boolean }> {
  return (await post("/votes/withdraw-partial", { appId, amount, userId })) as { withdrawn: boolean };
}

export async function fetchStakes(appId: string, userId: string): Promise<{ id: string; amount: number; appTagId: string }[]> {
  const res = (await get(`/stakes?appId=${encodeURIComponent(appId)}&userId=${encodeURIComponent(userId)}`)) as {
    stakes: { id: string; amount: number; appTagId: string }[];
  };
  return res.stakes;
}

export async function createStake(input: {
  appTagId: string;
  userId: string;
  amount: number;
  txSig: string | null;
  simulationMode: boolean;
}): Promise<{ stake: { id: string; amount: number } }> {
  return (await post("/stakes", input)) as { stake: { id: string; amount: number } };
}

export interface MyPosition {
  kind: "vote" | "tagStake";
  id: string;
  amount: number;
  appId: string;
  appSlug: string;
  appName: string;
  tagSlug: string | null;
  tagName: string | null;
}

/** Every active vote/tag-stake `userId` currently holds, across every app — see indexer/src/handlers/users.rs's get_positions. */
export async function fetchMyPositions(userId: string): Promise<MyPosition[]> {
  const { positions } = (await get(`/users/${encodeURIComponent(userId)}/positions`)) as {
    positions: MyPosition[];
  };
  return positions;
}

export async function withdrawStake(stakeId: string, userId: string): Promise<{ withdrawn: boolean }> {
  return (await post(`/stakes/${encodeURIComponent(stakeId)}/withdraw`, { userId })) as { withdrawn: boolean };
}

/** Withdraws `amount` off this (user, app-tag)'s active Stake rows, oldest-first — see indexer/src/handlers/stakes.rs's withdraw_partial. */
export async function withdrawPartialStakes(
  appTagId: string,
  amount: number,
  userId: string,
): Promise<{ withdrawn: boolean }> {
  return (await post("/stakes/withdraw-partial", { appTagId, amount, userId })) as { withdrawn: boolean };
}

export interface RewardsPositions {
  votes: { appId: string; appSlug: string; appName: string; appIconUrl: string | null; amount: number }[];
  stakes: {
    appTagId: string;
    appId: string;
    appSlug: string;
    appName: string;
    appIconUrl: string | null;
    tagSlug: string;
    tagName: string;
    amount: number;
  }[];
}

export async function fetchRewardsPositions(userId: string): Promise<RewardsPositions> {
  return (await get(`/rewards/positions?userId=${encodeURIComponent(userId)}`)) as RewardsPositions;
}

export interface UserXp {
  userId: string;
  xp: number;
  level: number;
  title: string;
  xpIntoLevel: number;
  xpForNextLevel: number;
  progress: number;
  appsSubmitted: number;
  tagsSuggested: number;
  votesCast: number;
  stakesMade: number;
  /** Task kinds already earned today (UTC) — excludes "daily_bonus", which isn't a user-initiated action. */
  xpEarnedToday: XpTaskKind[];
}

export type XpTaskKind = "submit_app" | "suggest_tag" | "vote" | "stake";

export async function fetchUserXp(userId: string): Promise<UserXp | null> {
  return (await getOrNull(`/xp/${encodeURIComponent(userId)}`)) as UserXp | null;
}

export interface XpActivityEntry {
  id: string;
  kind: "submit_app" | "suggest_tag" | "vote" | "stake" | "daily_bonus";
  appName: string | null;
  appSlug: string | null;
  tagName: string | null;
  amount: number;
  createdAt: string;
}

export async function fetchXpActivity(userId: string): Promise<XpActivityEntry[]> {
  const result = (await get(`/xp/${encodeURIComponent(userId)}/activity`)) as {
    events: XpActivityEntry[];
  };
  return result.events;
}

export interface XpLeaderboardEntry {
  userId: string;
  wallet: string;
  handle: string | null;
  xp: number;
  level: number;
  title: string;
}

export async function fetchXpLeaderboard(limit = 10): Promise<XpLeaderboardEntry[]> {
  const result = (await get(`/xp/leaderboard?limit=${encodeURIComponent(limit)}`)) as {
    entries: XpLeaderboardEntry[];
  };
  return result.entries;
}

export interface VisitorInfo {
  visitorId: string;
  sessionId: string;
  userAgent: string;
  path?: string;
  referrer?: string | null;
}

export async function serveAd(
  appId: string,
  visitor: VisitorInfo,
): Promise<{
  ad: { id: string; title: string; body: string; imageUrl: string | null; targetUrl: string } | null;
  impressionId?: string;
  reason?: string;
}> {
  return (await post("/ads/serve", { appId, ...visitor })) as {
    ad: { id: string; title: string; body: string; imageUrl: string | null; targetUrl: string } | null;
    impressionId?: string;
    reason?: string;
  };
}

export async function clickAd(impressionId: string): Promise<{ ok: boolean }> {
  return (await post("/ads/click", { impressionId })) as { ok: boolean };
}

export async function trackPageView(
  appId: string,
  visitor: VisitorInfo,
  revenueEligible: boolean,
): Promise<{ tracked: boolean; reason?: string; revenueEligible?: boolean }> {
  return (await post("/track", { appId, ...visitor, revenueEligible })) as {
    tracked: boolean;
    reason?: string;
    revenueEligible?: boolean;
  };
}

export async function writeDailySnapshot(): Promise<{ written: number }> {
  return (await post("/snapshots/daily", {})) as { written: number };
}

export interface MissingMetadataApp {
  id: string;
  slug: string;
  url: string;
  iconUrl: string | null;
  tagline: string;
  description: string;
}

export async function fetchAppsMissingMetadata(): Promise<MissingMetadataApp[]> {
  return (await get("/apps/missing-metadata")) as MissingMetadataApp[];
}

/** Revenue-eligible page-view count per app in `[start, end)` — see indexer/src/handlers/revenue.rs's `traffic`. */
export async function fetchPlatformTraffic(start: Date, end: Date): Promise<Record<string, number>> {
  const sp = new URLSearchParams({ start: start.toISOString(), end: end.toISOString() });
  return (await get(`/platform/traffic?${sp.toString()}`)) as Record<string, number>;
}

export async function updateAppMetadata(
  id: string,
  fields: { iconUrl?: string | null; tagline?: string; description?: string },
): Promise<void> {
  await patch(`/apps/${encodeURIComponent(id)}/metadata`, fields);
}

export interface X402SettleInput {
  signedTransaction: string;
  expectedAmountRaw: string;
  expectedMint: string;
  expectedPayTo: string;
}

/** Verifies + submits an x402 payment transaction — see indexer/src/handlers/x402.rs and app/src/lib/x402.ts. Throws (via post()'s error handling) if the transaction doesn't match what was expected or fails to land. */
export async function settleX402Payment(input: X402SettleInput): Promise<{ settled: boolean; transaction: string }> {
  return (await post("/x402/settle", input)) as { settled: boolean; transaction: string };
}
