import { type Page } from "@playwright/test";
import { expect, test } from "./fixtures";
import { nextQuestionCalls, resetStub } from "./stub";

/**
 * (b) of A83: handleAnswer pushes and handleBack pops/rewrites through the Next
 * router at all.
 *
 * Nothing short of a real browser has a history stack, so nothing short of one
 * can tell `router.back()` from `router.replace()` — and that distinction is
 * the entire point of backNavigation (A82).
 */

const Q1 = "Are you looking for a DeFi app?";
const Q2 = "Do you want to lend or borrow?";
const Q3 = "Does it have to be on Solana?";

function question(page: Page, prompt: string) {
  return page.getByRole("heading", { name: prompt });
}

function answer(page: Page, label: "Yes" | "No" | "Don't care") {
  return page.getByRole("button", { name: label, exact: true });
}

test.describe("answering and going back", () => {
  test("each answer pushes one history entry and browser Back steps back one question", async ({
    page,
  }) => {
    await resetStub();
    await page.goto("/find");
    await expect(question(page, Q1)).toBeVisible();

    await answer(page, "Yes").click();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes$/);
    await expect(question(page, Q2)).toBeVisible();

    await answer(page, "No").click();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes,tag:lending:no$/);
    await expect(question(page, Q3)).toBeVisible();

    // The browser's own Back, not the in-page control: this is the entry
    // handleAnswer pushed, and the effect has to notice the URL moved.
    await page.goBack();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes$/);
    await expect(question(page, Q2)).toBeVisible();
  });

  test("the in-page Back pops an entry this session pushed", async ({ page }) => {
    await resetStub();
    await page.goto("/find");
    await expect(question(page, Q1)).toBeVisible();
    await answer(page, "Yes").click();
    await expect(question(page, Q2)).toBeVisible();
    await answer(page, "No").click();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes,tag:lending:no$/);

    await page.getByRole("button", { name: "Back", exact: true }).click();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes$/);
    await expect(question(page, Q2)).toBeVisible();

    // A forward entry can only exist if Back POPPED. router.replace would have
    // overwritten the two-answer entry and left nothing to go forward to.
    await page.goForward();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes,tag:lending:no$/);
    await expect(question(page, Q3)).toBeVisible();
  });

  test("the in-page Back rewrites, and stays on the site, at a shared link's own depth", async ({
    page,
  }) => {
    await resetStub();
    // landingCount 1, answerCount 1 -> "rewrite". The entry underneath this one
    // belongs to whoever pasted the link, so popping it leaves the site — the
    // exact bug A82 exists to prevent, in reverse.
    await page.goto("/find?a=category:defi:yes");
    await expect(question(page, Q2)).toBeVisible();

    await page.getByRole("button", { name: "Back", exact: true }).click();

    await expect(page).toHaveURL(/\/find$/);
    await expect(page.getByTestId("find-funnel")).toBeVisible();
    await expect(question(page, Q1)).toBeVisible();
  });

  test("an answer costs exactly one engine round trip", async ({ page }) => {
    await resetStub();
    await page.goto("/find");
    await expect(question(page, Q1)).toBeVisible();

    await answer(page, "Yes").click();
    await expect(question(page, Q2)).toBeVisible();
    await answer(page, "No").click();
    await expect(question(page, Q3)).toBeVisible();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes,tag:lending:no$/);

    // A negative claim ("and nothing else was sent") needs a bound, so let any
    // straggler land before counting. Not a workaround for flakiness: under the
    // duplicate-request defect this guards, both calls are already in flight
    // before the question this test just waited for can render.
    await page.waitForTimeout(500);

    const calls = await nextQuestionCalls();
    // One server render plus one per answer. handleAnswer moves the reducer
    // BEFORE it pushes the URL precisely so that the effect then finds the two
    // in agreement and does not fetch the same question a second time; without
    // that, every answer costs two round trips against a force-dynamic route.
    expect(calls.map((c) => (c.body as { answers: unknown[] }).answers.length)).toEqual([0, 1, 2]);
  });

  test("re-answering a facet after Back revises it instead of stacking a second answer", async ({
    page,
  }) => {
    await resetStub();
    await page.goto("/find");
    await answer(page, "Yes").click();
    await expect(question(page, Q2)).toBeVisible();
    await answer(page, "No").click();
    await expect(question(page, Q3)).toBeVisible();

    await page.getByRole("button", { name: "Back", exact: true }).click();
    await expect(question(page, Q2)).toBeVisible();

    await answer(page, "Yes").click();
    await expect(page).toHaveURL(/\/find\?a=category:defi:yes,tag:lending:yes$/);
  });
});
