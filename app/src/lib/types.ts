// Data-transfer shapes returned by the API and consumed by the UI. Keeping
// these explicit decouples the client from the Prisma row shape.

export interface TagDTO {
  id: string; // AppTag id (app-scoped)
  tagId: string;
  slug: string;
  name: string;
  stakeTotal: number;
  suggestedBy: string | null;
}

/** How much an app's stats moved over a recent window — only populated on
    search results (see indexer/src/handlers/apps.rs's TrendDto), null/absent
    everywhere else. Each `*Pct` is independently null when its own baseline
    snapshot value was exactly 0 ("grew from 0" has no percent to show). */
export interface TrendDTO {
  intervalDays: number;
  voteWeightPct: number | null;
  stakeTotalPct: number | null;
  viewCountPct: number | null;
  rankScorePct: number | null;
}

export interface AppDTO {
  id: string;
  slug: string;
  name: string;
  tagline: string;
  description: string;
  url: string;
  iconUrl: string | null;
  category: string;
  chain: string;
  status: string;
  createdAt: string;
  submittedBy: string | null;
  voteCount: number;
  voteWeight: number;
  stakeTotal: number;
  viewCount: number;
  rankScore: number;
  tags: TagDTO[];
  trend?: TrendDTO | null;
}

export interface SearchResult {
  apps: AppDTO[];
  total: number;
  page: number;
  pageSize: number;
  facets: {
    tags: { slug: string; name: string; count: number }[];
  };
}

/** Full detail for a single app's page — see indexer/src/handlers/apps.rs's `AppDetailDto`. */
export interface AppDetail {
  app: AppDTO;
  recentVotes: {
    id: string;
    amount: number;
    createdAt: string;
    wallet: string;
    txSig: string | null;
  }[];
  topStakers: { wallet: string; amount: number }[];
  viewsLast7d: number;
  snapshots: {
    date: string;
    voteWeight: number;
    stakeTotal: number;
    viewCount: number;
    rankScore: number;
  }[];
}

// The /find funnel's wire vocabulary. Spellings are fixed by the Rust side
// (indexer/src/find/mod.rs) and must match it character for character.

export interface FacetRef {
  kind: "category" | "chain" | "tag";
  value: string;
}

export interface FindQuestion {
  facet: FacetRef;
  prompt: string;
}

export interface FindAnswer {
  facet: FacetRef;
  value: "yes" | "no" | "skip";
}

export interface FindShortlistEntry {
  app: AppDTO;
  /** Posterior probability this is the app the visitor wants, 0..1. */
  confidence: number;
}

export interface FindNextResult {
  question: FindQuestion | null;
  /** Empty unless `done` — a leak control, not a UI convenience. `/api/data/*`
      sells this catalog per request in NEB via x402, so returning ranked apps
      on every turn would let a caller sweep answer combinations and enumerate
      the catalog one HTTP call at a time. `candidateCount` gives the UI its
      progress signal without identities. */
  shortlist: FindShortlistEntry[];
  candidateCount: number;
  questionsAsked: number;
  /** `question` is null iff this is true. */
  done: boolean;
}

export interface FindNextInput {
  answers: FindAnswer[];
  forceResults?: boolean;
}

export interface FindConfirmInput {
  answers: FindAnswer[];
  appId: string;
  outcome: "confirmed" | "rejected" | "clicked";
  visitorId: string;
  sessionId: string;
}
