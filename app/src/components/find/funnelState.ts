import { dedupeAnswers, MAX_FORWARDED_ANSWERS } from "@/app/api/find/findRequest";
import { FIND_MAX_QUESTIONS } from "@/lib/constants";
import type { FacetRef, FindAnswer, FindNextResult } from "@/lib/types";

/**
 * The funnel's entire client-side bookkeeping, as a pure reducer.
 *
 * The engine itself lives in the indexer and the session is stateless on the
 * server (the client re-sends its whole answer history on every request), so
 * this answer list *is* the session. Keeping it a plain reducer means the
 * interesting behaviour — replace-in-place, back, restart, failure recovery —
 * is unit-testable with no DOM and no network.
 */
export interface FunnelState {
  answers: FindAnswer[];
  result: FindNextResult | null;
  loading: boolean;
  error: string | null;
}

export type FunnelAction =
  | { type: "answered"; facet: FacetRef; value: "yes" | "no" | "skip" }
  | { type: "restored"; answers: FindAnswer[] }
  | { type: "back" }
  | { type: "restart" }
  | { type: "loading" }
  | { type: "resolved"; result: FindNextResult }
  | { type: "failed"; message: string };

export const initialFunnelState: FunnelState = {
  answers: [],
  result: null,
  loading: false,
  error: null,
};

function sameFacet(a: FacetRef, b: FacetRef): boolean {
  return a.kind === b.kind && a.value === b.value;
}

export function funnelReducer(state: FunnelState, action: FunnelAction): FunnelState {
  switch (action.type) {
    case "answered": {
      const answer: FindAnswer = { facet: action.facet, value: action.value };
      const existing = state.answers.findIndex((a) => sameFacet(a.facet, action.facet));
      // Replaced in place rather than appended: going back and re-answering
      // must revise that answer, not stack a contradictory second one the
      // engine would then score twice.
      const answers =
        existing === -1
          ? [...state.answers, answer]
          : state.answers.map((a, i) => (i === existing ? answer : a));

      // `result` is dropped, not kept: the next question is about to be
      // fetched and leaving the current one rendered would show text the
      // visitor has already answered.
      return { answers, result: null, loading: true, error: null };
    }

    case "restored":
      return { answers: action.answers, result: null, loading: true, error: null };

    case "back": {
      if (state.answers.length === 0) return state;
      return {
        answers: state.answers.slice(0, -1),
        result: null,
        loading: true,
        error: null,
      };
    }

    case "restart":
      return initialFunnelState;

    case "loading":
      return { ...state, loading: true, error: null };

    case "resolved":
      return { ...state, result: action.result, loading: false, error: null };

    case "failed":
      // Deliberately preserves `answers` and `result`: a dropped request is
      // not a reason to throw away the questions the visitor already sat
      // through. Retrying re-sends the same history.
      return { ...state, error: action.message, loading: false };
  }
}

/** True once at least one question is answered — back, start over and
    "show me results" all unlock from that point on. */
export function canGoBack(state: FunnelState): boolean {
  return state.answers.length > 0;
}

/** Progress for the UI, 0..1. The server's `questionsAsked` is authoritative
    when present; mid-flight (result cleared, request in flight) the local
    answer count stands in so the bar doesn't drop back to zero between
    questions. */
export function funnelProgress(state: FunnelState): number {
  const asked = state.result?.questionsAsked ?? state.answers.length;
  return Math.min(1, asked / FIND_MAX_QUESTIONS);
}

/**
 * The configured Turnstile site key, or null when there isn't one.
 *
 * `app/.env.example` ships the var as `""` rather than leaving it unset, so an
 * emptiness check is the real test, not `=== undefined`. Null means no widget
 * renders and confirms post `turnstileToken: null` — which is exactly what
 * local and simulation mode need, since `TURNSTILE_SECRET_KEY` is unset there
 * too and the confirm route lets an unconfigured environment through.
 */
export function turnstileSiteKey(value: string | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

/** A Turnstile token is single-use: Cloudflare rejects one already redeemed,
    so a spent token is worth exactly as much as no token. See A61/A62. */
export interface TurnstileTokenState {
  token: string | null;
  spent: boolean;
}

export const initialTurnstileTokenState: TurnstileTokenState = {
  token: null,
  spent: false,
};

/** The token safe to attach to the next write, or null if there is none live. */
export function tokenToSend(state: TurnstileTokenState): string | null {
  return state.spent ? null : state.token;
}

/** After a write has redeemed the token: nothing left to send until the
    challenge mints a replacement. */
export function spendToken(): TurnstileTokenState {
  return { token: null, spent: true };
}

/** A fresh token from the challenge callback, clearing the spent flag. */
export function receiveToken(token: string): TurnstileTokenState {
  return { token, spent: false };
}

/**
 * Waits up to `timeoutMs` for the Turnstile challenge to produce a token,
 * resolving to null rather than hanging.
 *
 * The bound is the whole point: a site key not registered for the current
 * hostname, an ad-blocker on challenges.cloudflare.com, or a network hiccup
 * makes Turnstile's callback simply never fire. A confirm that waits forever
 * would leave "Is this the one?" dead under the visitor's finger, which is
 * worse than losing one telemetry row — so the timeout falls back to null and
 * the write goes out regardless.
 */
export function awaitTurnstileToken(
  read: () => string | null,
  timeoutMs: number,
  pollMs = 50,
): Promise<string | null> {
  const immediate = read();
  if (immediate !== null) return Promise.resolve(immediate);

  return new Promise((resolve) => {
    const startedAt = Date.now();
    const timer = setInterval(() => {
      const token = read();
      if (token !== null || Date.now() - startedAt >= timeoutMs) {
        clearInterval(timer);
        resolve(token);
      }
    }, pollMs);
  });
}

/**
 * The answer history mirrored into the query string.
 *
 * The server holds no session (A1), so the history the client carries IS the
 * session — and until it was written into the URL, a refresh threw it away and
 * a funnel path could not be shared. The format is deliberately legible rather
 * than an opaque blob, because these links get pasted to other people:
 *
 *     /find?a=category:defi:yes,tag:lending:no
 *
 * `:` and `,` are left literal (RFC 3986 allows both unescaped in a query);
 * only the facet value is percent-encoded, so a value containing a separator
 * still round-trips.
 */
export const FUNNEL_ANSWERS_PARAM = "a";

const FACET_KINDS: readonly string[] = ["category", "chain", "tag"];
const ANSWER_VALUES: readonly string[] = ["yes", "no", "skip"];

/** Mirrors findFacetSchema's own `.max(64)` in lib/validation.ts — a URL may
    not describe a facet the route would reject. */
const MAX_FACET_VALUE_LENGTH = 64;

function isFacetKind(value: string): value is FacetRef["kind"] {
  return FACET_KINDS.includes(value);
}

function isAnswerValue(value: string): value is FindAnswer["value"] {
  return ANSWER_VALUES.includes(value);
}

export function encodeFunnelAnswers(answers: FindAnswer[]): string {
  return answers
    .map((a) => `${a.facet.kind}:${encodeURIComponent(a.facet.value)}:${a.value}`)
    .join(",");
}

/** Empty history gets the bare `/find` — the one URL that stays indexable
    (A5), so a fresh funnel never looks like a parameterized near-duplicate. */
export function findFunnelHref(answers: FindAnswer[]): string {
  const encoded = encodeFunnelAnswers(answers);
  return encoded ? `/find?${FUNNEL_ANSWERS_PARAM}=${encoded}` : "/find";
}

function parseEntry(entry: string): FindAnswer | null {
  const parts = entry.split(":");
  if (parts.length !== 3) return null;

  const [kind, encodedValue, answer] = parts;
  if (!isFacetKind(kind) || !isAnswerValue(answer)) return null;

  let value: string;
  try {
    value = decodeURIComponent(encodedValue);
  } catch {
    // A truncated "%" escape. decodeURIComponent throws URIError on it, and an
    // unhandled throw here would blank the page over a mangled link.
    return null;
  }
  if (value.length === 0 || value.length > MAX_FACET_VALUE_LENGTH) return null;

  return { facet: { kind, value }, value: answer };
}

/**
 * A query string is input, not state: parsed as far as it holds up, discarded
 * from the first entry that does not.
 *
 * Truncating rather than skipping the bad entry is the point — a string that
 * stops making sense was most likely cut short in transit, and its sound
 * prefix is a state this funnel genuinely passed through. Keeping the entries
 * on the far side of the damage would instead assemble an answer path nobody
 * ever walked and train the learned term on it.
 */
export function parseFunnelAnswers(raw: string | null | undefined): FindAnswer[] {
  if (!raw) return [];

  const parsed: FindAnswer[] = [];
  for (const entry of raw.split(",")) {
    const answer = parseEntry(entry);
    if (answer === null) break;
    parsed.push(answer);
  }

  // Same normalization the route applies to the body it forwards, reached for
  // rather than re-implemented: a facet named twice would otherwise be scored
  // twice, and an unbounded history is a free way to make the engine re-score
  // the catalog N times per request.
  return dedupeAnswers(parsed).slice(0, MAX_FORWARDED_ANSWERS);
}

function sameAnswers(a: FindAnswer[], b: FindAnswer[]): boolean {
  return (
    a.length === b.length &&
    a.every((x, i) => sameFacet(x.facet, b[i].facet) && x.value === b[i].value)
  );
}

export interface UrlSyncDeps {
  /** Adopt this history into the reducer. */
  restore: (answers: FindAnswer[]) => void;
  /** Show the spinner and ask the engine for the question at this history. */
  request: (answers: FindAnswer[]) => void;
  /** Rewrite the address bar in place, adding no history entry. */
  replaceUrl: (href: string) => void;
}

/**
 * Makes the funnel agree with the URL — the whole of mount, refresh, browser
 * Back and browser Forward, in one decision.
 *
 * Given injected dependencies for the same reason submitOutcome has them
 * (A66): the interesting claims here are about what gets ASKED FOR — which
 * answers reach the reducer and which body reaches /api/find/next — and a test
 * that walked the parser by hand would assert none of that.
 */
export function reconcileUrlAnswers(
  deps: UrlSyncDeps,
  raw: string | null,
  state: FunnelState,
): void {
  const restored = parseFunnelAnswers(raw);

  // Whatever the page went on to do has to be what the address bar says, so a
  // URL we could only partly honour is rewritten to the part we honoured.
  // `replace`, not `push`: correcting a bad link is not a step in the funnel.
  if ((raw ?? "") !== encodeFunnelAnswers(restored)) {
    deps.replaceUrl(findFunnelHref(restored));
  }

  if (!sameAnswers(restored, state.answers)) {
    deps.restore(restored);
    deps.request(restored);
    return;
  }

  // Already in agreement — which is the common case, since answering pushes
  // the URL the reducer has just moved to. The only reason left to fetch is
  // having no question to show and none in flight: the server could not
  // render the first one because the indexer was unreachable.
  if (state.result === null && !state.loading) deps.request(restored);
}

/**
 * Where Back goes. `landingCount` is how many answers the URL already carried
 * when the page loaded: those history entries belong to whoever shared the
 * link, so popping past them would leave the site instead of stepping back a
 * question — which is the bug this whole mirror exists to fix, in reverse.
 */
export function backNavigation(
  answerCount: number,
  landingCount: number,
): "none" | "pop" | "rewrite" {
  if (answerCount === 0) return "none";
  return answerCount > landingCount ? "pop" : "rewrite";
}

export interface OutcomeBody {
  answers: FindAnswer[];
  appId: string;
  outcome: "confirmed" | "rejected";
  turnstileToken: string | null;
}

export interface OutcomeDeps {
  /** The live, unredeemed token, or null. */
  readToken: () => string | null;
  /** Burn the redeemed token and ask the widget for a replacement. */
  cycleToken: () => void;
  post: (body: OutcomeBody) => Promise<void>;
  /** False in local/simulation mode, where no site key is configured and the
      confirm route accepts an unverified write. */
  turnstileEnabled: boolean;
  timeoutMs: number;
  pollMs?: number;
}

/**
 * One "Is this the one?" outcome: acquire a token, post, cycle.
 *
 * Extracted from the component and given injected dependencies so the token
 * lifecycle can be asserted on the actual request bodies. Asserting on the
 * helpers alone proved nothing — a test that walks receiveToken/spendToken by
 * hand stays green even if the component never calls them (A66).
 */
export async function submitOutcome(
  deps: OutcomeDeps,
  answers: FindAnswer[],
  appId: string,
  outcome: "confirmed" | "rejected",
): Promise<void> {
  const turnstileToken = deps.turnstileEnabled
    ? await awaitTurnstileToken(deps.readToken, deps.timeoutMs, deps.pollMs)
    : null;

  try {
    await deps.post({ answers, appId, outcome, turnstileToken });
  } finally {
    // `finally`, not the success path: /api/find/confirm verifies the token
    // BEFORE it writes, so a token attached to a request that 403s or 500s
    // has already been consumed at Cloudflare. Leaving it in place would make
    // every later outcome re-send a dead token and 403 in turn, stranding the
    // session — reintroducing exactly the biased-sample loss A62 closed.
    // Still gated on a token having been sent: on the tokenless path there is
    // nothing to burn, and resetting would cancel the in-flight challenge
    // about to mint the token the next outcome needs (A63).
    if (turnstileToken !== null) deps.cycleToken();
  }
}
