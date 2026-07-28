#!/usr/bin/env bash
# End-to-end tier for /find (A83): a real chromium against a real Next server,
# with the indexer replaced by e2e/stubIndexer.ts.
#
# Needs Node and network access for the browser binary. No secrets, no
# database, no Solana RPC, no indexer build — the stub is a ~150-line node
# server and the funnel is read-only.
#
# Playwright's own `webServer` starts and stops both the stub and `next dev`,
# so this script only has to guarantee the browser is present and hand over.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -d node_modules/@playwright/test ]; then
  echo "find-e2e: @playwright/test is missing — run 'npm install' first" >&2
  exit 1
fi

# Idempotent: a no-op once the binary is cached. This is the one extra step CI
# needs beyond `npm ci`, and it is why the job wants a cache on
# ~/.cache/ms-playwright (~/Library/Caches/ms-playwright on macOS).
if [ -z "${PLAYWRIGHT_SKIP_BROWSER_INSTALL:-}" ]; then
  npx playwright install chromium
fi

exec npx playwright test "$@"
