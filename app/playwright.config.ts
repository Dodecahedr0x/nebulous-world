import { tmpdir } from "node:os";
import { join } from "node:path";
import { defineConfig, devices, type PlaywrightTestConfig } from "@playwright/test";

// `*.e2e.ts`, not `*.spec.ts`: vitest's default `include` is
// `**/*.{test,spec}.?(c|m)[jt]s?(x)`, so a file named `.spec.ts` here would be
// collected by `npm test` too and fail outside a Playwright runner. This
// extension keeps the two suites apart without touching vitest.config.ts.
const TEST_MATCH = "**/*.e2e.ts";

const APP_PORT = process.env.E2E_APP_PORT || "3100";
const STUB_PORT = process.env.E2E_STUB_PORT || "8099";
const BASE_URL = `http://127.0.0.1:${APP_PORT}`;
const STUB_URL = `http://127.0.0.1:${STUB_PORT}`;

// `next dev`, not `next build && next start`: /find is force-dynamic, so it is
// rendered per request under both, and dev is the only one that does not first
// have to build ~40 other routes — several of which prerender against the
// indexer this run replaces with a stub. Dev also hot-reloads, which is what
// lets the mutation battery flip a guard and re-run without a server restart.
// The cost is React StrictMode's double effect invocation; the specs assert on
// request CONTENT and on exact zero-counts rather than on exact positive
// counts, so they hold either way.
const webServer: PlaywrightTestConfig["webServer"] = process.env.E2E_EXTERNAL_SERVERS
  ? undefined
  : [
      {
        command: `npx tsx e2e/stubIndexer.ts`,
        url: `${STUB_URL}/__requests`,
        env: { STUB_PORT },
        reuseExistingServer: true,
        stdout: "pipe" as const,
      },
      {
        command: `npx next dev --hostname 127.0.0.1 --port ${APP_PORT}`,
        // Hitting /find rather than / also warms the route's first compile,
        // which in dev is slower than anything the specs themselves do.
        url: `${BASE_URL}/find`,
        env: {
          INDEXER_API_URL: STUB_URL,
          // Empty, not unset: turnstileSiteKey trims to null on "" and the
          // widget never mounts, so nothing in this suite reaches Cloudflare.
          NEXT_PUBLIC_TURNSTILE_SITE_KEY: "",
          NEXT_TELEMETRY_DISABLED: "1",
        },
        timeout: 240_000,
        reuseExistingServer: true,
        stdout: "pipe" as const,
      },
    ];

export default defineConfig({
  testDir: "./e2e",
  testMatch: TEST_MATCH,
  // One stub process holds one shared request recorder, and every spec resets
  // it — parallel workers would read each other's requests.
  workers: 1,
  fullyParallel: false,
  // A retry would hide a flake behind a green tick, and this suite exists to
  // prove specs are not vacuous.
  retries: 0,
  forbidOnly: !!process.env.CI,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [["list"]],
  // Traces and screenshots go to the system temp dir, not app/test-results:
  // app/.gitignore is outside this node's scope, so an in-repo artifact
  // directory would show up as untracked junk in every later `git status`.
  outputDir: join(tmpdir(), "find-e2e-artifacts"),
  use: {
    baseURL: BASE_URL,
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer,
});
