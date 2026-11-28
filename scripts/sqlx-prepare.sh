#!/usr/bin/env bash
# Regenerate `.sqlx/`, the offline data the `sqlx::query!` family compiles
# against, or with `--check` fail when it is stale.
#
# The database is SQLX_PREPARE_DATABASE_URL, which this migrates, so name a
# disposable one. Unset, a throwaway Postgres starts and stops here. Never
# DATABASE_URL: it may name a shared database.
#
# Usage: scripts/sqlx-prepare.sh [--check]

set -euo pipefail
cd "$(dirname "$0")/.."

CHECK=()
case "${1:-}" in
  "") ;;
  --check) CHECK=(--check) ;;
  *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac

log() { echo "[sqlx-prepare] $*" >&2; }

if ! cargo sqlx --version >/dev/null 2>&1; then
  log "cargo-sqlx is missing; install the version Cargo.lock pins:"
  log "  cargo install sqlx-cli@0.9.0 --locked --no-default-features --features postgres,rustls"
  exit 1
fi

URL="${SQLX_PREPARE_DATABASE_URL:-}"
if [ -z "$URL" ]; then
  # shellcheck source=scripts/lib/postgres-bin.sh
  . scripts/lib/postgres-bin.sh
  BIN="$(pg_bin_dir)"
  PG_ROOT="$(mktemp -d)"
  PG_DATA="$PG_ROOT/data"
  trap '"$BIN/pg_ctl" -D "$PG_DATA" -m immediate stop >/dev/null 2>&1 || true; rm -rf "$PG_ROOT"' EXIT
  PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
  "$BIN/initdb" -D "$PG_DATA" -U postgres --auth=trust -E UTF8 --no-sync >/dev/null
  "$BIN/pg_ctl" -D "$PG_DATA" -l "$PG_ROOT/postgres.log" -w \
    -o "-h 127.0.0.1 -p $PORT -k $PG_ROOT -c fsync=off" start >/dev/null
  URL="postgres://postgres@127.0.0.1:$PORT/postgres"
  log "Started a throwaway Postgres on 127.0.0.1:$PORT"
fi

# `sqlx migrate run` is the migrator `moatless_core::db::run_migrations` wraps;
# on an empty database its baseline adoption and lock retries do nothing.
cargo sqlx migrate run --source crates/core/migrations --database-url "$URL"

# No `RUSTC_WRAPPER`: sccache keys a crate on its source, not on the database
# or `.sqlx/`, so a hit skips the macro and its query file is never written.
# The `cargo check` targets match the clippy floor, which has already built
# the dependencies' metadata.
RUSTC_WRAPPER="" cargo sqlx prepare --workspace "${CHECK[@]+"${CHECK[@]}"}" \
  --database-url "$URL" -- --workspace --all-targets
