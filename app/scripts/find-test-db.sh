#!/usr/bin/env bash
# Provisions the Postgres that the /find live-database test tier needs, and
# sweeps the databases a failed run leaked.
#
# Usage, from the repo root:
#
#   bash app/scripts/find-test-db.sh
#   cd indexer && FIND_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres \
#     cargo test -p nebulous-world-indexer -- --ignored
#
# The argument (default postgresql://postgres:postgres@localhost:5432/postgres)
# is an ADMIN url: indexer/src/find_integration.rs connects to it only to
# CREATE/DROP one `find_it_<unique>` database per test, so no test ever sees
# another's rows and the tier runs in parallel. A test that panics cannot drop
# its own database (a Drop impl cannot await), which is what the sweep below is
# for — it is not a tidiness measure, it is the leak's only cleanup path.
#
# CI does NOT run this script. It delegates to ensure-postgres.sh, which is
# Homebrew/macOS-specific by design and exits 1 on a machine with no brew; and
# a fresh runner has no leaked database to sweep. CI instead declares a
# `postgres:15` service container with POSTGRES_PASSWORD=postgres (user and db
# both default to `postgres`), needs no secrets, and runs the two cargo
# commands below from indexer/:
#
#   cargo test -p nebulous-world-indexer
#   FIND_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres \
#     cargo test -p nebulous-world-indexer -- --ignored
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

ADMIN_URL="${1:-${FIND_TEST_DATABASE_URL:-postgresql://postgres:postgres@localhost:5432/postgres}}"

# Delegates rather than reimplements: install/start postgresql@15 and create the
# role and database named in the url.
./scripts/ensure-postgres.sh "$ADMIN_URL"

if command -v brew >/dev/null 2>&1 && brew list --formula postgresql@15 >/dev/null 2>&1; then
  PATH="$(brew --prefix postgresql@15)/bin:$PATH"
  export PATH
fi
if ! command -v psql >/dev/null 2>&1; then
  echo "error: psql not on PATH — cannot sweep leaked test databases." >&2
  exit 1
fi

LEAKED="$(psql "$ADMIN_URL" -tAc \
  "SELECT datname FROM pg_database WHERE datname LIKE 'find\_it\_%'")"
for db in $LEAKED; do
  echo "==> Dropping leaked test database '$db'"
  psql "$ADMIN_URL" -q -c "DROP DATABASE IF EXISTS \"$db\" WITH (FORCE);"
done

echo "==> Ready. Run:"
echo "    cd indexer && FIND_TEST_DATABASE_URL=$ADMIN_URL cargo test -p nebulous-world-indexer -- --ignored"
