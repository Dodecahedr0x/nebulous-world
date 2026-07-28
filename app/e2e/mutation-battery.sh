#!/usr/bin/env bash
# Mutation battery for the /find end-to-end tier (A74).
#
# For every guard in e2e/mutants.py: break it, run the whole suite, expect the
# suite to go RED, restore. A guard whose mutant leaves the suite green is a
# hole in the tier, reported as SURVIVED rather than papered over.
#
# app/src is mutated and restored, never modified: a byte-level checksum of the
# whole tree is taken before the first mutant and re-checked at the end, which
# is strictly stronger than `git diff` here because every /find source file is
# still untracked in this worktree and `git diff` cannot see it at all.
set -uo pipefail

cd "$(dirname "$0")/.."

APP_PORT="${E2E_APP_PORT:-3100}"
STUB_PORT="${E2E_STUB_PORT:-8099}"
BASE_URL="http://127.0.0.1:${APP_PORT}"
STUB_URL="http://127.0.0.1:${STUB_PORT}"
LOG_DIR="$(mktemp -d)"

# Exclusive lock. Two batteries on one tree interleave mutations and silently
# corrupt each other's verdicts — mkdir is the atomicity primitive, succeeding
# for exactly one caller. Release lives in cleanup() so it survives a kill.
LOCK_DIR="${TMPDIR:-/tmp}/find-e2e-mutation.lock"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  holder="$(cat "$LOCK_DIR/owner" 2>/dev/null || echo unknown)"
  if [ "$holder" != unknown ] && ! kill -0 "$holder" 2>/dev/null; then
    echo "clearing stale mutation lock from dead pid $holder" >&2
    rm -rf "$LOCK_DIR"
    mkdir "$LOCK_DIR" || exit 1
  else
    echo "REFUSING: another mutation run holds the lock (pid $holder)." >&2
    echo "  Two batteries on one tree corrupt each other's verdicts." >&2
    echo "  Wait for it, or if certain it is dead: rm -rf $LOCK_DIR" >&2
    exit 1
  fi
fi
echo $$ > "$LOCK_DIR/owner"
# Lets this run's own mutants.py calls through the guard in that file.
export FIND_E2E_LOCK_OWNER=$$

# Guards this tier CANNOT kill, each for a stated reason. Listed so that a new
# survivor is a failure rather than a line of output nobody reads.
#
#   G18 handleBack's `if (step === "none") return;` — unreachable through the
#       UI, because QuestionCard only renders Back when canGoBack (i.e. at
#       least one answer), and backNavigation returns "none" only at zero.
#
# G17 was on this list and should not have been (A94 supersedes A93): deleting
# handleAnswer's dispatch does not merely cost a frame, it costs a second
# /find/next per answer, which the stub's recorder sees directly. "an answer
# costs exactly one engine round trip" now kills it.
EXPECTED_SURVIVORS="G18"

tree_checksum() {
  find src -type f -print0 | sort -z | xargs -0 shasum | shasum | awk '{print $1}'
}

cleanup() {
  python3 e2e/mutants.py restore >/dev/null 2>&1
  rm -rf "$LOCK_DIR"
  [ -n "${NEXT_PID:-}" ] && kill "$NEXT_PID" 2>/dev/null
  [ -n "${STUB_PID:-}" ] && kill "$STUB_PID" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

BEFORE="$(tree_checksum)"
echo "app/src checksum before: $BEFORE"

npx tsx e2e/stubIndexer.ts >"$LOG_DIR/stub.log" 2>&1 &
STUB_PID=$!
INDEXER_API_URL="$STUB_URL" NEXT_PUBLIC_TURNSTILE_SITE_KEY="" NEXT_TELEMETRY_DISABLED=1 \
  npx next dev --hostname 127.0.0.1 --port "$APP_PORT" >"$LOG_DIR/next.log" 2>&1 &
NEXT_PID=$!

echo -n "waiting for servers"
for _ in $(seq 1 120); do
  if curl -fsS "$STUB_URL/__requests" >/dev/null 2>&1 &&
    curl -fsS "$BASE_URL/find" >/dev/null 2>&1; then
    break
  fi
  echo -n "."
  sleep 2
done
echo

run_suite() {
  # E2E_EXTERNAL_SERVERS: this script owns the two servers, so `next dev` keeps
  # its process (and therefore hot-reloads each mutant) across all runs.
  E2E_EXTERNAL_SERVERS=1 npx playwright test --reporter=line >"$1" 2>&1
}

echo "=== baseline (unmutated) ==="
if ! run_suite "$LOG_DIR/baseline.log"; then
  echo "BASELINE FAILED — a battery on a red suite proves nothing" >&2
  tail -30 "$LOG_DIR/baseline.log" >&2
  exit 1
fi
tail -2 "$LOG_DIR/baseline.log"

KILLED=0
SURVIVED=0
SURVIVORS=""
IDS="$(python3 e2e/mutants.py list | awk '{print $1}')"

for id in $IDS; do
  what="$(python3 e2e/mutants.py list | awk -F'\t' -v k="$id" '$1==k {print $3}')"
  python3 e2e/mutants.py apply "$id" || exit 1
  # Force the dev server to recompile the mutated module before the browser
  # asks for it.
  sleep 1
  curl -fsS "$BASE_URL/find" >/dev/null 2>&1
  sleep 1

  if run_suite "$LOG_DIR/$id.log"; then
    SURVIVED=$((SURVIVED + 1))
    SURVIVORS="$SURVIVORS $id"
    printf '%-5s SURVIVED  %s\n' "$id" "$what"
  else
    KILLED=$((KILLED + 1))
    printf '%-5s killed by %-9s %s\n' "$id" \
      "$(grep -Eo '[0-9]+ failed' "$LOG_DIR/$id.log" | tail -1)" "$what"
    grep -E '^\s+[0-9]+\) ' "$LOG_DIR/$id.log" | sed 's/^/        /' | head -6
  fi

  python3 e2e/mutants.py restore >/dev/null
  sleep 1
  curl -fsS "$BASE_URL/find" >/dev/null 2>&1
done

TOTAL=$((KILLED + SURVIVED))
echo
echo "guards enumerated: $TOTAL   killed: $KILLED   survived: $SURVIVED  [$SURVIVORS ]"

AFTER="$(tree_checksum)"
echo "app/src checksum after:  $AFTER"
if [ "$BEFORE" != "$AFTER" ]; then
  echo "app/src NOT RESTORED" >&2
  exit 1
fi
echo "app/src restored byte-for-byte"

echo "=== final (restored) ==="
run_suite "$LOG_DIR/final.log" || {
  echo "suite red after restore" >&2
  tail -30 "$LOG_DIR/final.log" >&2
  exit 1
}
tail -2 "$LOG_DIR/final.log"
echo "logs: $LOG_DIR"

# Set equality, both ways: an unexpected survivor is a coverage hole, and an
# expected survivor that suddenly dies means the list is stale.
if [ "$(echo $SURVIVORS | tr ' ' '\n' | sort | tr '\n' ' ')" != \
  "$(echo $EXPECTED_SURVIVORS | tr ' ' '\n' | sort | tr '\n' ' ')" ]; then
  echo "survivor set changed — expected [$EXPECTED_SURVIVORS], got [$SURVIVORS ]" >&2
  exit 1
fi
echo "survivors match the documented set [$EXPECTED_SURVIVORS]"
