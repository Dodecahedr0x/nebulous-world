#!/usr/bin/env bash
# Winds down everything scripts/setup-dev.sh starts: the local surfpool
# Surfnet, the indexer (and the dlmm-bridge sidecar it spawns), and the
# local Postgres instance (see scripts/ensure-postgres.sh). Safe to run
# even if some/none are up.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
ROOT_DIR="$(cd .. && pwd)"

SURFPOOL_PID_FILE="$ROOT_DIR/.surfpool/surfpool.pid"
RPC_PORT=8899
INDEXER_PID_FILE="$ROOT_DIR/.surfpool/indexer.pid"
INDEXER_API_PORT=8090
DLMM_BRIDGE_PORT=8091

log() { printf '\n\033[1;36m==> %s\033[0m\n' "$1"; }

stop_by_pid_or_port() {
  local label="$1" pid_file="$2" port="$3"
  local pid=""
  if [ -n "$pid_file" ] && [ -f "$pid_file" ]; then
    pid="$(cat "$pid_file")"
  fi
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid"
    echo "  $label stopped (pid $pid)"
  # -sTCP:LISTEN matters: without it lsof also matches every *client* connected
  # to the port — the Next.js dev server talking to the indexer on 8090, say —
  # and this would kill those instead of the server that owns the port.
  elif lsof -ti ":$port" -sTCP:LISTEN >/dev/null 2>&1; then
    lsof -ti ":$port" -sTCP:LISTEN | xargs kill
    echo "  $label stopped (found on port $port)"
  else
    echo "  $label not running"
  fi
  [ -n "$pid_file" ] && rm -f "$pid_file"
}

log "Stopping surfpool"
stop_by_pid_or_port "surfpool" "$SURFPOOL_PID_FILE" "$RPC_PORT"

log "Stopping the indexer and dlmm-bridge"
stop_by_pid_or_port "indexer" "$INDEXER_PID_FILE" "$INDEXER_API_PORT"
# dlmm-bridge is a child process the indexer spawns (see
# indexer/src/dlmm_bridge.rs) — killing the indexer's own pid doesn't
# propagate to it, so it's stopped independently by its own port.
stop_by_pid_or_port "dlmm-bridge" "" "$DLMM_BRIDGE_PORT"

log "Stopping local Postgres (postgresql@15)"
if ! command -v brew >/dev/null 2>&1 || ! brew list --formula postgresql@15 >/dev/null 2>&1; then
  echo "  not managed by Homebrew, skipping"
elif brew services list 2>/dev/null | grep -q '^postgresql@15 *started'; then
  brew services stop postgresql@15 >/dev/null
  echo "  stopped"
else
  echo "  not running"
fi

log "Done"
