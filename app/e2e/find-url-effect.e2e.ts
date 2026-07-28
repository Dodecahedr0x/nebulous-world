import { expect, test } from "./fixtures";
import { nextQuestionCalls, resetStub } from "./stub";

/**
 * (a) of A83: FindFunnel's one useEffect really calls reconcileUrlAnswers, and
 * really wires `restore`, `request` and `replaceUrl` to the reducer, the API
 * and the router.
 *
 * The pure function is exhaustively unit-tested; none of that proves the
 * component ever reaches it. Only a real browser evaluates useSearchParams and
 * router.replace, which is why this tier is a browser and not a fetch.
 */
test.describe("the URL drives the funnel", () => {
  test("a one-answer URL restores that answer and asks for the question after it", async ({
    page,
  }) => {
    await resetStub();
    await page.goto("/find?a=category:defi:yes");

    await expect(page.getByTestId("find-funnel")).toBeVisible();
    // The stub returns a different facet per turn, so the second question's
    // text is only reachable by having SENT one answer.
    await expect(
      page.getByRole("heading", { name: "Do you want to lend or borrow?" }),
    ).toBeVisible();
    await expect(page.getByText("Question 2 of up to 8")).toBeVisible();
    // Back only renders when the reducer holds an answer (canGoBack), so its
    // presence is the proof that `restore` reached the reducer rather than
    // `request` having quietly carried the history on its own.
    await expect(page.getByRole("button", { name: "Back", exact: true })).toBeVisible();

    const calls = await nextQuestionCalls();
    expect(calls.length).toBeGreaterThan(0);
    for (const call of calls) {
      // `forceResults: false` is findNextSchema's default, applied by the Next
      // route — its presence is how a client-driven call is distinguishable
      // from page.tsx's own server-side one.
      expect(call.body).toEqual({
        answers: [{ facet: { kind: "category", value: "defi" }, value: "yes" }],
        forceResults: false,
      });
    }
  });

  test("the server-rendered first question is not re-fetched by the client on mount", async ({
    page,
  }) => {
    await resetStub();
    await page.goto("/find");

    await expect(
      page.getByRole("heading", { name: "Are you looking for a DeFi app?" }),
    ).toBeVisible();
    // Exactly the server render's call and nothing else: the effect must find
    // the URL and the seeded state already in agreement. A second call here is
    // the wasted round trip per answer that `resuming` exists to prevent, and
    // it would also mean `initialResult` never reached the reducer.
    expect(await nextQuestionCalls()).toHaveLength(1);
  });

  test("a URL that only partly parses is corrected in place, not pushed", async ({ page }) => {
    await resetStub();

    await page.goto("/find");
    const baseline = await page.evaluate(() => history.length);

    await page.goto("/find?a=category:defi:yes,garbage");

    // Truncate-not-skip (A82): the sound prefix survives, the damage and
    // everything after it does not.
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes$/);
    await expect(
      page.getByRole("heading", { name: "Do you want to lend or borrow?" }),
    ).toBeVisible();

    // `replace`, not `push` — correcting a bad link is not a step in the
    // funnel, and a pushed correction would trap Back on the broken URL.
    // +1 is the goto itself; anything more is a history entry the effect added.
    expect(await page.evaluate(() => history.length)).toBe(baseline + 1);
  });

  test("a wholly unparseable answer string falls back to a bare /find", async ({ page }) => {
    await resetStub();
    await page.goto("/find?a=not-an-answer");

    await expect(page).toHaveURL(/\/find$/);
    await expect(
      page.getByRole("heading", { name: "Are you looking for a DeFi app?" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Back", exact: true })).toHaveCount(0);
  });
});
