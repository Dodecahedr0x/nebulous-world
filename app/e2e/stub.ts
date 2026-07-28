import { expect, type Page } from "@playwright/test";
import type { RecordedRequest } from "./stubIndexer";

export const STUB_URL = process.env.STUB_URL || "http://127.0.0.1:8099";

/**
 * Clears the recorder — after a short drain, deliberately.
 *
 * Playwright closes the previous test's context between tests, but a request
 * that already left the browser is still on its way through the Next server,
 * and this suite's sharpest assertions are negative ("the stub saw NOTHING").
 * Draining first means a straggler lands before the clear rather than after it,
 * so a leaked request can never be misread as the request under test.
 */
export async function resetStub(drainMs = 250): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, drainMs));
  const res = await fetch(`${STUB_URL}/__reset`, { method: "POST" });
  expect(res.ok, "stub indexer must be reachable — start it with e2e/stubIndexer.ts").toBe(true);
}

export async function stubRequests(): Promise<RecordedRequest[]> {
  const res = await fetch(`${STUB_URL}/__requests`);
  return (await res.json()) as RecordedRequest[];
}

export async function nextQuestionCalls(): Promise<RecordedRequest[]> {
  const all = await stubRequests();
  return all.filter((r) => r.method === "POST" && r.path === "/find/next");
}

/** The `a=` value of the page's current URL, or null when there is none. */
export function answersParam(page: Page): string | null {
  return new URL(page.url()).searchParams.get("a");
}
