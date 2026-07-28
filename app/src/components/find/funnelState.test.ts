import { describe, it, expect } from "vitest";
import { FIND_MAX_QUESTIONS } from "@/lib/constants";
import type { FacetRef, FindAnswer, FindNextResult } from "@/lib/types";
import {
  initialFunnelState,
  funnelReducer,
  canGoBack,
  funnelProgress,
  turnstileSiteKey,
  awaitTurnstileToken,
  initialTurnstileTokenState,
  tokenToSend,
  spendToken,
  receiveToken,
  submitOutcome,
  encodeFunnelAnswers,
  findFunnelHref,
  reconcileUrlAnswers,
  backNavigation,
  FUNNEL_ANSWERS_PARAM,
  type FunnelState,
  type OutcomeBody,
} from "@/components/find/funnelState";
import { generateMetadata } from "@/app/find/page";

const LENDING: FacetRef = { kind: "tag", value: "lending" };
const DEFI_TAG: FacetRef = { kind: "tag", value: "defi" };
const DEFI_CATEGORY: FacetRef = { kind: "category", value: "defi" };

function result(over: Partial<FindNextResult> = {}): FindNextResult {
  return {
    question: { facet: LENDING, prompt: "Are you after lending?" },
    shortlist: [],
    candidateCount: 41,
    questionsAsked: 1,
    done: false,
    ...over,
  };
}

describe("initialFunnelState", () => {
  it("starts empty, idle and unresolved", () => {
    expect(initialFunnelState.answers).toEqual([]);
    expect(initialFunnelState.result).toBeNull();
    expect(initialFunnelState.loading).toBe(false);
    expect(initialFunnelState.error).toBeNull();
  });
});

describe("answered", () => {
  it("appends one answer and enters loading", () => {
    const next = funnelReducer(initialFunnelState, {
      type: "answered",
      facet: LENDING,
      value: "yes",
    });
    expect(next.answers).toEqual([{ facet: LENDING, value: "yes" }]);
    expect(next.loading).toBe(true);
  });

  it("replaces an already-answered facet in place instead of duplicating it", () => {
    const first = funnelReducer(initialFunnelState, {
      type: "answered",
      facet: LENDING,
      value: "yes",
    });
    const second = funnelReducer(first, {
      type: "answered",
      facet: DEFI_CATEGORY,
      value: "no",
    });
    const revised = funnelReducer(second, {
      type: "answered",
      facet: LENDING,
      value: "no",
    });

    expect(revised.answers).toHaveLength(2);
    expect(revised.answers[0]).toEqual({ facet: LENDING, value: "no" });
    expect(revised.answers[1]).toEqual({ facet: DEFI_CATEGORY, value: "no" });
  });

  it("treats facets sharing a value but differing in kind as distinct", () => {
    const first = funnelReducer(initialFunnelState, {
      type: "answered",
      facet: DEFI_TAG,
      value: "yes",
    });
    const second = funnelReducer(first, {
      type: "answered",
      facet: DEFI_CATEGORY,
      value: "no",
    });

    expect(second.answers).toEqual([
      { facet: DEFI_TAG, value: "yes" },
      { facet: DEFI_CATEGORY, value: "no" },
    ]);
  });

  it("clears a previously-resolved result so no stale question stays on screen", () => {
    const resolved: FunnelState = {
      answers: [],
      result: result(),
      loading: false,
      error: "boom",
    };
    const next = funnelReducer(resolved, {
      type: "answered",
      facet: LENDING,
      value: "skip",
    });

    expect(next.result).toBeNull();
    expect(next.error).toBeNull();
    expect(next.loading).toBe(true);
  });
});

describe("back", () => {
  it("drops the last answer and clears the result", () => {
    const first = funnelReducer(initialFunnelState, {
      type: "answered",
      facet: LENDING,
      value: "yes",
    });
    const second = funnelReducer(first, {
      type: "answered",
      facet: DEFI_CATEGORY,
      value: "no",
    });
    const resolved = funnelReducer(second, { type: "resolved", result: result() });
    const back = funnelReducer(resolved, { type: "back" });

    expect(back.answers).toEqual([{ facet: LENDING, value: "yes" }]);
    expect(back.result).toBeNull();
    expect(back.loading).toBe(true);
  });

  it("is a no-op on an empty answer history", () => {
    expect(() => funnelReducer(initialFunnelState, { type: "back" })).not.toThrow();
    expect(funnelReducer(initialFunnelState, { type: "back" })).toEqual(initialFunnelState);
  });
});

describe("restart", () => {
  it("returns a state deep-equal to the initial one", () => {
    const populated: FunnelState = {
      answers: [
        { facet: LENDING, value: "yes" },
        { facet: DEFI_CATEGORY, value: "no" },
      ],
      result: result({ done: true, questionsAsked: 4 }),
      loading: true,
      error: "network down",
    };

    expect(funnelReducer(populated, { type: "restart" })).toEqual(initialFunnelState);
  });
});

describe("failed", () => {
  it("keeps the answers and the result so a transient failure costs nothing", () => {
    const populated: FunnelState = {
      answers: [{ facet: LENDING, value: "yes" }],
      result: result(),
      loading: true,
      error: null,
    };
    const failed = funnelReducer(populated, { type: "failed", message: "network down" });

    expect(failed.answers).toEqual(populated.answers);
    expect(failed.result).toEqual(populated.result);
    expect(failed.error).toBe("network down");
    expect(failed.loading).toBe(false);
  });
});

describe("resolved", () => {
  it("clears loading and any earlier error", () => {
    const failing: FunnelState = {
      answers: [{ facet: LENDING, value: "yes" }],
      result: null,
      loading: true,
      error: "network down",
    };
    const next = funnelReducer(failing, { type: "resolved", result: result() });

    expect(next.result).toEqual(result());
    expect(next.loading).toBe(false);
    expect(next.error).toBeNull();
  });
});

describe("canGoBack", () => {
  it("is false initially and true once one question is answered", () => {
    expect(canGoBack(initialFunnelState)).toBe(false);
    const answered = funnelReducer(initialFunnelState, {
      type: "answered",
      facet: LENDING,
      value: "yes",
    });
    expect(canGoBack(answered)).toBe(true);
  });
});

describe("funnelProgress", () => {
  it("is 0 initially", () => {
    expect(funnelProgress(initialFunnelState)).toBe(0);
  });

  it("caps at 1 when questionsAsked runs past the hard cap", () => {
    const overrun: FunnelState = {
      answers: [],
      result: result({ questionsAsked: FIND_MAX_QUESTIONS + 5 }),
      loading: false,
      error: null,
    };
    expect(funnelProgress(overrun)).toBe(1);
  });

  it("reports the fraction of the cap already asked", () => {
    const midway: FunnelState = {
      answers: [],
      result: result({ questionsAsked: 2 }),
      loading: false,
      error: null,
    };
    expect(funnelProgress(midway)).toBeCloseTo(2 / FIND_MAX_QUESTIONS);
  });
});

describe("turnstileSiteKey", () => {
  // .env.example ships the var as "" rather than omitting it, so the empty
  // case is the one local and simulation mode actually hit.
  it("is null when unset, empty or whitespace — the no-widget path", () => {
    expect(turnstileSiteKey(undefined)).toBeNull();
    expect(turnstileSiteKey("")).toBeNull();
    expect(turnstileSiteKey("   ")).toBeNull();
  });

  it("returns the trimmed key when one is configured", () => {
    expect(turnstileSiteKey("0x4AAA")).toBe("0x4AAA");
    expect(turnstileSiteKey("  0x4AAA  ")).toBe("0x4AAA");
  });
});

describe("awaitTurnstileToken", () => {
  it("resolves immediately when the token has already arrived", async () => {
    await expect(awaitTurnstileToken(() => "tok", 1000)).resolves.toBe("tok");
  });

  it("resolves the token once the challenge fires late", async () => {
    let token: string | null = null;
    setTimeout(() => {
      token = "late-tok";
    }, 20);

    await expect(awaitTurnstileToken(() => token, 1000, 5)).resolves.toBe("late-tok");
  });

  it("falls back to null rather than hanging when the challenge never resolves", async () => {
    // The site-key-not-registered-for-this-hostname case: the callback never
    // fires, so the confirm must go out tokenless instead of blocking.
    await expect(awaitTurnstileToken(() => null, 30, 5)).resolves.toBeNull();
  });
});

describe("turnstile token cycling (A61/A62)", () => {
  it("starts with no token to send", () => {
    expect(initialTurnstileTokenState).toEqual({ token: null, spent: false });
    expect(tokenToSend(initialTurnstileTokenState)).toBeNull();
  });

  it("offers a freshly minted token", () => {
    expect(tokenToSend(receiveToken("tok-1"))).toBe("tok-1");
  });

  it("never offers a token that has already been redeemed", () => {
    // Cloudflare rejects a re-used token, so a spent one is worth exactly as
    // much as no token: null. Offering it again would 403 the write.
    const spent = spendToken();
    expect(spent.token).toBeNull();
    expect(spent.spent).toBe(true);
    expect(tokenToSend(spent)).toBeNull();
  });

  it("composes with awaitTurnstileToken so a spent token waits for the re-mint", async () => {
    let state = spendToken();
    setTimeout(() => {
      state = receiveToken("tok-fresh");
    }, 20);

    await expect(
      awaitTurnstileToken(() => tokenToSend(state), 1000, 5),
    ).resolves.toBe("tok-fresh");
  });

  it("gives up on a re-mint that never arrives instead of blocking the button", async () => {
    const state = spendToken();
    await expect(awaitTurnstileToken(() => tokenToSend(state), 30, 5)).resolves.toBeNull();
  });
});

describe("submitOutcome — token lifecycle observed on the request bodies", () => {
  // Stands in for the Cloudflare widget: mints a fresh token whenever the
  // previous one is cycled, exactly as `turnstile.reset()` causes the render
  // callback to fire again.
  function fakeWidget() {
    let state = initialTurnstileTokenState;
    let minted = 0;
    const mint = () => {
      state = receiveToken(`tok-${(minted += 1)}`);
    };
    mint();
    return {
      readToken: () => tokenToSend(state),
      cycleToken: () => {
        state = spendToken();
        mint();
      },
    };
  }

  function recorder() {
    const bodies: OutcomeBody[] = [];
    return { bodies, post: async (b: OutcomeBody) => void bodies.push(b) };
  }

  it("'Not quite' then 'Yes' carry two different live tokens (A62)", async () => {
    // The regression itself. Without the cycle, both posts carry tok-1 and
    // the confirm — the corrective, highest-information row — 403s.
    const widget = fakeWidget();
    const { bodies, post } = recorder();
    const deps = { ...widget, post, turnstileEnabled: true, timeoutMs: 500, pollMs: 1 };

    await submitOutcome(deps, [], "app-a", "rejected");
    await submitOutcome(deps, [], "app-b", "confirmed");

    expect(bodies).toHaveLength(2);
    expect(bodies[0]).toMatchObject({ appId: "app-a", outcome: "rejected", turnstileToken: "tok-1" });
    expect(bodies[1]).toMatchObject({ appId: "app-b", outcome: "confirmed", turnstileToken: "tok-2" });
    expect(bodies[1].turnstileToken).not.toBe(bodies[0].turnstileToken);
  });

  it("cycles after a FAILED post, because verification consumes the token before the write", async () => {
    // /api/find/confirm verifies then writes, so a 403 still burned the
    // token. If the cycle sat on the success path the next outcome would
    // re-send a dead token and 403 in turn, stranding the session.
    const widget = fakeWidget();
    const sent: OutcomeBody[] = [];

    await expect(
      submitOutcome(
        {
          ...widget,
          post: async (b) => {
            sent.push(b);
            throw new Error("Verification failed");
          },
          turnstileEnabled: true,
          timeoutMs: 500,
          pollMs: 1,
        },
        [],
        "app-a",
        "rejected",
      ),
    ).rejects.toThrow("Verification failed");

    const { bodies, post } = recorder();
    await submitOutcome(
      { ...widget, post, turnstileEnabled: true, timeoutMs: 500, pollMs: 1 },
      [],
      "app-b",
      "confirmed",
    );

    expect(sent[0].turnstileToken).toBe("tok-1");
    expect(bodies[0].turnstileToken).toBe("tok-2");
  });

  it("does not cycle when no token was attached (A63)", async () => {
    // Nothing to burn, and resetting would cancel the in-flight challenge
    // about to mint the token the next outcome needs.
    let cycles = 0;
    const { bodies, post } = recorder();

    await submitOutcome(
      {
        readToken: () => null,
        cycleToken: () => void (cycles += 1),
        post,
        turnstileEnabled: false,
        timeoutMs: 20,
        pollMs: 1,
      },
      [],
      "app-a",
      "confirmed",
    );

    expect(bodies[0].turnstileToken).toBeNull();
    expect(cycles).toBe(0);
  });

  it("posts tokenless rather than blocking when the challenge never resolves", async () => {
    let cycles = 0;
    const { bodies, post } = recorder();

    await submitOutcome(
      {
        readToken: () => null,
        cycleToken: () => void (cycles += 1),
        post,
        turnstileEnabled: true,
        timeoutMs: 20,
        pollMs: 1,
      },
      [],
      "app-a",
      "confirmed",
    );

    expect(bodies[0].turnstileToken).toBeNull();
    expect(cycles).toBe(0);
  });

  it("carries the full answer history, since the server holds no session (A1)", async () => {
    const widget = fakeWidget();
    const { bodies, post } = recorder();
    const answers: FindAnswer[] = [
      { facet: LENDING, value: "yes" },
      { facet: DEFI_CATEGORY, value: "no" },
    ];

    await submitOutcome(
      { ...widget, post, turnstileEnabled: true, timeoutMs: 500, pollMs: 1 },
      answers,
      "app-a",
      "confirmed",
    );

    expect(bodies[0].answers).toEqual(answers);
  });
});

/**
 * The URL mirror (A82).
 *
 * Every assertion below goes through `reconcileUrlAnswers` — the same seam
 * FindFunnel's one URL effect calls — and asserts on what that seam ASKED FOR:
 * the answers handed to the reducer, the body the engine would be sent, the
 * href the address bar is rewritten to. Nothing here hand-walks the parser and
 * checks its return value; that is the inert shape A74 was written about.
 */
function urlRecorder() {
  const restored: FindAnswer[][] = [];
  const requested: FindAnswer[][] = [];
  const replaced: string[] = [];
  return {
    restored,
    requested,
    replaced,
    deps: {
      restore: (a: FindAnswer[]) => void restored.push(a),
      request: (a: FindAnswer[]) => void requested.push(a),
      replaceUrl: (href: string) => void replaced.push(href),
    },
  };
}

/** A funnel sitting on question 1 with nothing answered — a cold page load. */
function fresh(over: Partial<FunnelState> = {}): FunnelState {
  return { answers: [], result: result(), loading: false, error: null, ...over };
}

describe("resuming from the query string", () => {
  it("asks the engine for exactly the history the link carried", () => {
    const r = urlRecorder();
    // Pinned as a literal, not as encodeFunnelAnswers' output: this string is
    // the shared-link format itself, and a test that re-derives it would agree
    // with any encoding the code happened to emit.
    reconcileUrlAnswers(r.deps, "category:defi:yes,tag:lending:no,chain:solana:skip", fresh());

    const expected: FindAnswer[] = [
      { facet: { kind: "category", value: "defi" }, value: "yes" },
      { facet: { kind: "tag", value: "lending" }, value: "no" },
      { facet: { kind: "chain", value: "solana" }, value: "skip" },
    ];
    expect(r.restored).toEqual([expected]);
    expect(r.requested).toEqual([expected]);
    expect(r.replaced).toEqual([]);
  });

  it("emits that same format, so a link this session shares resumes", () => {
    const answers: FindAnswer[] = [
      { facet: { kind: "category", value: "defi" }, value: "yes" },
      { facet: LENDING, value: "no" },
    ];
    expect(encodeFunnelAnswers(answers)).toBe("category:defi:yes,tag:lending:no");
    expect(findFunnelHref(answers)).toBe("/find?a=category:defi:yes,tag:lending:no");
  });

  it("leaves the bare /find unparameterized, so it stays the indexable one (A5)", () => {
    expect(findFunnelHref([])).toBe("/find");
    expect(encodeFunnelAnswers([])).toBe("");
  });

  it("round-trips a facet value carrying the separators themselves", () => {
    const odd: FindAnswer[] = [{ facet: { kind: "tag", value: "a,b:c" }, value: "yes" }];
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, encodeFunnelAnswers(odd), fresh());

    expect(r.requested).toEqual([odd]);
    // Percent-encoded on the way out, so the separators stay unambiguous.
    expect(r.replaced).toEqual([]);
  });

  it("does nothing at all when the URL already agrees with the funnel", () => {
    const r = urlRecorder();
    const answers: FindAnswer[] = [{ facet: LENDING, value: "yes" }];
    reconcileUrlAnswers(r.deps, "tag:lending:yes", fresh({ answers }));

    expect(r.restored).toEqual([]);
    expect(r.requested).toEqual([]);
    expect(r.replaced).toEqual([]);
  });

  it("fetches the first question when the server could not render one", () => {
    // The indexer was unreachable at request time, so `initialResult` is null
    // and the client has to ask for question 1 itself.
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, null, fresh({ result: null }));

    expect(r.requested).toEqual([[]]);
  });

  it("does not fire a second request while one is already in flight", () => {
    // The turn the visitor just answered: the reducer has already cleared the
    // result and pushed the new URL. Re-requesting here would double every
    // answer's round trip.
    const r = urlRecorder();
    const answers: FindAnswer[] = [{ facet: LENDING, value: "yes" }];
    reconcileUrlAnswers(r.deps, "tag:lending:yes", fresh({ answers, result: null, loading: true }));

    expect(r.requested).toEqual([]);
  });

  it("does not re-fetch a question already on screen", () => {
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, null, fresh());

    expect(r.requested).toEqual([]);
  });

  it("adopts a revised answer to the same question", () => {
    // Back, then answer differently: the reducer replaces in place, so the
    // history stays the same length. Comparing lengths alone would leave the
    // visitor looking at the old branch of the funnel.
    const r = urlRecorder();
    const answers: FindAnswer[] = [{ facet: LENDING, value: "yes" }];
    reconcileUrlAnswers(r.deps, "tag:lending:no", fresh({ answers }));

    expect(r.requested).toEqual([[{ facet: LENDING, value: "no" }]]);
  });

  it("adopts a history that asked a different question altogether", () => {
    const r = urlRecorder();
    const answers: FindAnswer[] = [{ facet: LENDING, value: "yes" }];
    reconcileUrlAnswers(r.deps, "category:defi:yes", fresh({ answers }));

    expect(r.requested).toEqual([[{ facet: { kind: "category", value: "defi" }, value: "yes" }]]);
  });

  it("asks once, not twice, when stepping back from a funnel with no question", () => {
    // The awkward combination: a request failed (no result, nothing in
    // flight) and the visitor then hit Back. Adopting the shorter history and
    // then also taking the have-no-question path would double the round trip.
    const r = urlRecorder();
    const answers: FindAnswer[] = [{ facet: LENDING, value: "yes" }];
    reconcileUrlAnswers(r.deps, null, fresh({ answers, result: null, error: "network down" }));

    expect(r.requested).toEqual([[]]);
  });

  it("steps back to the shorter history when the browser pops an entry", () => {
    const r = urlRecorder();
    const answers: FindAnswer[] = [
      { facet: { kind: "category", value: "defi" }, value: "yes" },
      { facet: LENDING, value: "no" },
    ];
    reconcileUrlAnswers(r.deps, "category:defi:yes", fresh({ answers }));

    const expected = [{ facet: { kind: "category", value: "defi" }, value: "yes" }];
    expect(r.restored).toEqual([expected]);
    expect(r.requested).toEqual([expected]);
  });
});

describe("a query string that was edited or truncated", () => {
  // Not one of these may crash, and none may put an answer the engine's
  // vocabulary does not contain into the body sent to /api/find/next.
  const REJECTED: [string, string][] = [
    ["a facet kind we do not have", "category:defi:yes,wallet:phantom:no"],
    ["an answer outside yes|no|skip (A15)", "category:defi:yes,tag:lending:maybe"],
    ["a missing segment", "category:defi:yes,tag:lending"],
    ["an extra segment", "category:defi:yes,tag:lending:no:extra"],
    ["an empty facet value", "category:defi:yes,tag::no"],
    ["a facet value past the 64-char schema bound", `category:defi:yes,tag:${"x".repeat(65)}:no`],
    ["a truncated percent escape", "category:defi:yes,tag:%:no"],
  ];

  it.each(REJECTED)("keeps the sound prefix and drops the rest: %s", (_label, raw) => {
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, raw, fresh());

    const kept = [{ facet: { kind: "category", value: "defi" }, value: "yes" }];
    expect(r.requested).toEqual([kept]);
    expect(r.restored).toEqual([kept]);
    // What we actually applied is what the address bar must say.
    expect(r.replaced).toEqual(["/find?a=category:defi:yes"]);
  });

  it("truncates at the damage rather than stitching the far side back on", () => {
    // Skipping the bad entry and keeping what follows would invent an answer
    // path the visitor never walked — the two survivors were never adjacent.
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, "category:defi:yes,BROKEN,tag:lending:no", fresh());

    expect(r.requested).toEqual([[{ facet: { kind: "category", value: "defi" }, value: "yes" }]]);
  });

  it("falls back to a fresh funnel when the whole string is junk", () => {
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, "%%%", fresh());

    expect(r.restored).toEqual([]);
    expect(r.requested).toEqual([]);
    expect(r.replaced).toEqual(["/find"]);
  });

  it("scores a facet the URL names twice exactly once, latest answer winning", () => {
    // The engine re-scores the whole history each turn (A1); a facet present
    // twice would be counted twice, which is the contradiction dedupeAnswers
    // exists to prevent on the server side.
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, "tag:lending:yes,category:defi:no,tag:lending:skip", fresh());

    expect(r.requested).toEqual([
      [
        { facet: LENDING, value: "skip" },
        { facet: { kind: "category", value: "defi" }, value: "no" },
      ],
    ]);
  });

  it("never sends more answers than the route forwards (A41)", () => {
    const raw = Array.from({ length: 40 }, (_, i) => `tag:t${i}:yes`).join(",");
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, raw, fresh());

    expect(r.requested[0]).toHaveLength(16);
    expect(r.requested[0][15]).toEqual({ facet: { kind: "tag", value: "t15" }, value: "yes" });
  });

  it("does not rewrite a URL that is already canonical", () => {
    const r = urlRecorder();
    reconcileUrlAnswers(r.deps, "category:defi:yes", fresh());

    expect(r.replaced).toEqual([]);
  });
});

describe("backNavigation", () => {
  it("does nothing with no answer to step back over", () => {
    expect(backNavigation(0, 0)).toBe("none");
  });

  it("pops an entry this session pushed", () => {
    expect(backNavigation(3, 0)).toBe("pop");
    expect(backNavigation(3, 2)).toBe("pop");
  });

  it("rewrites instead of popping off the end of a shared link", () => {
    // Landing on /find?a=<two answers>, the entry beneath is whatever the
    // visitor was browsing before — popping it would leave the site rather
    // than step back a question.
    expect(backNavigation(2, 2)).toBe("rewrite");
    expect(backNavigation(1, 2)).toBe("rewrite");
  });
});

describe("restored", () => {
  it("adopts the history and re-enters loading, dropping any stale question", () => {
    const answers: FindAnswer[] = [{ facet: LENDING, value: "yes" }];
    const next = funnelReducer(fresh({ error: "boom" }), { type: "restored", answers });

    expect(next.answers).toEqual(answers);
    expect(next.result).toBeNull();
    expect(next.loading).toBe(true);
    expect(next.error).toBeNull();
  });
});

describe("/find metadata (A5)", () => {
  it("leaves the bare page indexable", async () => {
    const meta = await generateMetadata({ searchParams: Promise.resolve({}) });
    expect(meta.robots).toBeUndefined();
    expect(meta.alternates?.canonical).toContain("/find");
  });

  it("noindexes a resumed funnel URL and canonicalizes it back to /find", async () => {
    // Load-bearing now that the funnel writes its state into the query string:
    // every answered turn is a distinct URL, and without this each would be a
    // near-duplicate of /find competing with it.
    const meta = await generateMetadata({
      searchParams: Promise.resolve({ [FUNNEL_ANSWERS_PARAM]: "category:defi:yes,tag:lending:no" }),
    });

    expect(meta.robots).toEqual({ index: false, follow: true });
    expect(meta.alternates?.canonical).toContain("/find");
  });
});
