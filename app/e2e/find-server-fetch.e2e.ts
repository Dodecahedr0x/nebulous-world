import { expect, test } from "./fixtures";
import { nextQuestionCalls, resetStub } from "./stub";

/**
 * (c) of A83: /find/page.tsx skips its server-side first-question fetch when
 * the URL is resuming.
 *
 * Deliberately at the HTTP level with a bare `fetch` and no browser: nothing
 * client-side runs, so the only thing that can have reached the stub is the
 * server render. That isolates the short-circuit from the funnel's own mount
 * fetch, which a browser test cannot tell apart.
 */
test.describe("page.tsx `resuming` short-circuit", () => {
  test("a bare /find asks the indexer for question 1 during the server render", async ({
    baseURL,
  }) => {
    await resetStub();

    const res = await fetch(`${baseURL}/find`);
    expect(res.status).toBe(200);
    expect(await res.text()).toContain('data-testid="find-funnel"');

    const calls = await nextQuestionCalls();
    expect(calls.length).toBeGreaterThan(0);
    // The empty history is the whole claim: the server asks for question 1.
    for (const call of calls) expect(call.body).toEqual({ answers: [] });
  });

  test("a resuming /find?a=… makes no server-side call at all", async ({ baseURL }) => {
    await resetStub();

    const res = await fetch(`${baseURL}/find?a=category:defi:yes`);
    expect(res.status).toBe(200);
    // The page still rendered — this is a skipped fetch, not a failed one.
    expect(await res.text()).toContain('data-testid="find-funnel"');

    expect(await nextQuestionCalls()).toEqual([]);
  });

  test("an unrelated query string does not count as resuming", async ({ baseURL }) => {
    await resetStub();

    // `resuming` keys on FUNNEL_ANSWERS_PARAM specifically, not on "the URL has
    // parameters" — generateMetadata's isParameterized is the one that keys on
    // the latter, and conflating the two would silently stop serving the first
    // question to anything arriving with a utm tag.
    const res = await fetch(`${baseURL}/find?utm_source=e2e`);
    expect(res.status).toBe(200);

    const calls = await nextQuestionCalls();
    expect(calls.length).toBeGreaterThan(0);
    for (const call of calls) expect(call.body).toEqual({ answers: [] });
  });
});
