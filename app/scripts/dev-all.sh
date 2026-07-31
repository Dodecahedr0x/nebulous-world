#!/usr/bin/env bash
# One-command local dev: tear down whatever a previous session left behind,
# run the full environment setup (scripts/setup-dev.sh — surfpool, program
# deploy, NEB launch, the indexer, which applies its own database schema on
# startup), then start the Next.js dev server. Ctrl-C (or any exit) tears
# everything back
# down via scripts/teardown-dev.sh, so you don't have to remember a
# separate command.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

DEV_PORT="${PORT:-3000}"

# Start from a clean slate. setup-dev.sh reuses anything already listening on
# the surfpool/indexer ports, so a crashed or orphaned previous session (stale
# surfpool holding a dead Surfnet's state, an indexer built from older code)
# would otherwise be silently adopted by this one.
echo
bash scripts/teardown-dev.sh
# teardown-dev.sh only knows about what setup-dev.sh starts; the dev server is
# started here, so it's killed here. Left running, `next dev` would quietly
# pick port 3001 instead. -sTCP:LISTEN matters: without it lsof also matches
# every *client* connected to the port, i.e. the browser tab you have open on
# localhost:3000, and this would kill that instead.
DEV_PIDS="$(lsof -ti ":$DEV_PORT" -sTCP:LISTEN 2>/dev/null || true)"
if [ -n "$DEV_PIDS" ]; then
  # shellcheck disable=SC2086 # word splitting is intended: one pid per line
  kill $DEV_PIDS 2>/dev/null || true
  echo "  dev server stopped (found on port $DEV_PORT)"
fi

cleanup() {
  echo
  bash scripts/teardown-dev.sh
}
trap cleanup EXIT
trap exit INT TERM

bash scripts/setup-dev.sh
npm run dev
