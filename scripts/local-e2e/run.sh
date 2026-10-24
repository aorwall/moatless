#!/usr/bin/env bash
# run.sh — Run the E2E suite against a throwaway backend inside this sandbox.
#
# The backend runs from this checkout with SANDBOX_PROVIDER=local, so every
# task's sandbox is a child process here rather than a pod. Its data lives in a
# fresh schema on the deployment's own Postgres, the way soap previews share one
# database; the role cannot create databases.
#
# Usage:
#   scripts/local-e2e/run.sh [--skip-build] [--keep] [-- <playwright args>]
#   scripts/local-e2e/run.sh sweep [--older-than-hours N] [--dry-run]
set -euo pipefail

cd "$(dirname "$0")/../.."
REPO_ROOT="$(pwd)"
RUNTIME_DIR="${RUNTIME_DIR:-/run/moatless}"
BASE_ENV="${LOCAL_E2E_BASE_ENV:-${RUNTIME_DIR}/shadow/backend.env}"
TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
# Where `moatless-backend`, `sandbox` and `moat` are; CI builds `--profile ci`.
BIN_DIR="${LOCAL_E2E_BIN_DIR:-${TARGET_DIR}/debug}"
# Only names this script mints: `e2e_<epoch seconds>_<random>`.
SCHEMA_PATTERN='^e2e_[0-9]+_[a-z0-9]+$'

log() { echo "[local-e2e] $*" >&2; }

# DATABASE_URL from the base env, without leaking the rest of it into this shell.
base_database_url() {
  if [ ! -f "$BASE_ENV" ]; then
    log "Syncing the deployment's backend env into ${BASE_ENV}"
    scripts/shadow/sync-env.sh "$BASE_ENV"
  fi
  # shellcheck disable=SC1090
  # Drop any inherited `options=`: it outranks PGOPTIONS and would pin psql to
  # another schema.
  (set -a; source "$BASE_ENV"; printf '%s' "${DATABASE_URL:?DATABASE_URL missing from $BASE_ENV}") |
    sed -E 's/([?&])options=[^&]*(&|$)/\1/; s/[?&]$//'
}

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'
}

# Pids whose environment names the run's sandbox root: the backend
# (`LOCAL_SANDBOX_ROOT`), each sandbox (`HOME`, `TMPDIR`), and every agent and
# command they started, whatever session or group those moved to.
run_pids() {
  local root="$1" proc
  for proc in /proc/[0-9]*; do
    [ "${proc#/proc/}" != "$$" ] || continue
    # An `if`, so a last non-match cannot fail the caller's `$(...)` under `set -e`.
    if grep -qzE "=${root}(/|$)" "${proc}/environ" 2>/dev/null; then
      echo "${proc#/proc/}"
    fi
  done
}

# SIGTERM the run's processes, then SIGKILL what is left after 10s.
stop_run() {
  local root="$1" pids
  pids="$(run_pids "$root")"
  [ -n "$pids" ] || return 0
  # shellcheck disable=SC2086
  kill -TERM $pids 2>/dev/null || true
  for _ in $(seq 1 10); do
    [ -n "$(run_pids "$root")" ] || return 0
    sleep 1
  done
  # shellcheck disable=SC2086
  kill -KILL $(run_pids "$root") 2>/dev/null || true
}

sweep() {
  local older_than_hours=3 dry_run=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --older-than-hours) older_than_hours="$2"; shift 2 ;;
      --dry-run) dry_run=1; shift ;;
      *) log "unknown sweep option: $1"; exit 2 ;;
    esac
  done
  local db cutoff schema epoch
  db="$(base_database_url)"
  cutoff=$(( $(date +%s) - older_than_hours * 3600 ))
  for schema in $(psql "$db" -Atc "SELECT nspname FROM pg_namespace WHERE nspname LIKE 'e2e\_%'"); do
    [[ "$schema" =~ $SCHEMA_PATTERN ]] || continue
    epoch="${schema#e2e_}"; epoch="${epoch%%_*}"
    [ "$epoch" -lt "$cutoff" ] || continue
    if [ "$dry_run" = 1 ]; then
      log "would stop /tmp/le-${schema#e2e_}'s processes and drop ${schema}"
    else
      log "stopping /tmp/le-${schema#e2e_}'s processes and dropping ${schema}"
      stop_run "/tmp/le-${schema#e2e_}"
      psql "$db" -qc "DROP SCHEMA \"${schema}\" CASCADE"
    fi
  done
  local dir
  for dir in /tmp/le-[0-9]*_* "${RUNTIME_DIR}"/local-e2e/e2e_[0-9]*_*; do
    [ -d "$dir" ] || continue
    epoch="${dir##*/}"; epoch="${epoch#le-}"; epoch="${epoch#e2e_}"; epoch="${epoch%%_*}"
    [[ "$epoch" =~ ^[0-9]+$ ]] && [ "$epoch" -lt "$cutoff" ] || continue
    if [ "$dry_run" = 1 ]; then
      log "would remove ${dir}"
    else
      log "removing ${dir}"
      case "$dir" in /tmp/le-*) stop_run "$dir" ;; esac
      rm -rf "$dir"
    fi
  done
}

if [ "${1:-}" = "sweep" ]; then
  shift
  sweep "$@"
  exit 0
fi

SKIP_BUILD=0
KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --keep) KEEP=1; shift ;;
    --) shift; break ;;
    *) log "unknown option: $1"; exit 2 ;;
  esac
done

DB_URL="$(base_database_url)"
SCHEMA="e2e_$(date +%s)_$(head -c 4 /dev/urandom | od -An -tx1 | tr -d ' \n')"
# Short: the sandbox's credential socket under it must fit a 108-byte path.
SANDBOX_ROOT="/tmp/le-${SCHEMA#e2e_}"
RUN_DIR="${RUNTIME_DIR}/local-e2e/${SCHEMA}"
# Names the base env does not set: the subshell below sources it.
HTTP_PORT="$(free_port)"
RPC_PORT="$(free_port)"
PROM_PORT="$(free_port)"
BASE_URL="http://127.0.0.1:${HTTP_PORT}"
RUN_ADMIN_EMAIL="admin@localhost"
RUN_ADMIN_PASSWORD="$(head -c 18 /dev/urandom | base64 | tr -d '/+=')"
BACKEND_PID=""

cleanup() {
  local status=$?
  if [ -n "$BACKEND_PID" ]; then
    stop_run "$SANDBOX_ROOT"
    # Reaped here, or bash reports the kill as if the backend had crashed.
    wait "$BACKEND_PID" 2>/dev/null || true
  fi
  if [ "$KEEP" = 1 ]; then
    log "Kept schema ${SCHEMA}, ${SANDBOX_ROOT} and ${RUN_DIR}"
  else
    psql "$DB_URL" -qc "DROP SCHEMA IF EXISTS \"${SCHEMA}\" CASCADE" || log "could not drop ${SCHEMA}; run: $0 sweep"
    rm -rf "$SANDBOX_ROOT" "$RUN_DIR"
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

mkdir -p "$SANDBOX_ROOT" "$RUN_DIR"
chmod 700 "$RUN_DIR"

if [ "$SKIP_BUILD" = 0 ]; then
  log "Building backend, sandbox and CLI"
  cargo build -p moatless-backend -p moatless-sandbox -p moatless-cli
fi

log "Creating schema ${SCHEMA}"
psql "$DB_URL" -qc "CREATE SCHEMA \"${SCHEMA}\""

# `search_path` in the URL scopes every pooled connection to the schema.
case "$DB_URL" in *\?*) SEP='&' ;; *) SEP='?' ;; esac
RUN_DB_URL="${DB_URL}${SEP}options=-c%20search_path%3D${SCHEMA}"

log "Starting backend on ${BASE_URL} (log: ${RUN_DIR}/backend.log)"
(
  set -a
  # shellcheck disable=SC1090
  source "$BASE_ENV"
  DATABASE_URL="$RUN_DB_URL"
  RUN_MIGRATIONS=true
  # The shadow env disables this; this run's dispatch queue is its own schema's,
  # and without it no agent turn ever completes.
  EVENT_HANDLER_ENABLED=true
  DATABASE_MAX_CONNECTIONS=5
  PORT="$HTTP_PORT"
  GRPC_PORT="$RPC_PORT"
  METRICS_PORT="$PROM_PORT"
  FRONTEND_URL="$BASE_URL"
  COOKIE_DOMAIN=
  SANDBOX_PROVIDER=local
  LOCAL_SANDBOX_ROOT="$SANDBOX_ROOT"
  LOCAL_SANDBOX_BIN="${BIN_DIR}/sandbox"
  AUTH_PROVIDER=local
  # Claude Code refuses to skip permissions under root (CI's container) without it.
  IS_SANDBOX=1
  ADMIN_USERNAME=admin
  ADMIN_EMAIL="$RUN_ADMIN_EMAIL"
  ADMIN_PASSWORD="$RUN_ADMIN_PASSWORD"
  set +a
  exec "${BIN_DIR}/moatless-backend"
) > "${RUN_DIR}/backend.log" 2>&1 &
BACKEND_PID=$!

for _ in $(seq 1 180); do
  if curl -fsS "${BASE_URL}/health" > /dev/null 2>&1; then break; fi
  if ! kill -0 "$BACKEND_PID" 2>/dev/null; then
    tail -20 "${RUN_DIR}/backend.log" >&2
    log "Backend exited before it was healthy"
    exit 1
  fi
  sleep 1
done
curl -fsS "${BASE_URL}/health" > /dev/null || {
  tail -30 "${RUN_DIR}/backend.log" >&2
  log "Backend not healthy after 180s"
  exit 1
}

# An API-key auth.json satisfies the Codex connection check without an OAuth
# refresh token. Its placeholder JWTs never expire, so the backend never tries
# to refresh them.
if [ -n "${OPENAI_API_KEY:-}" ]; then
  log "Seeding a global Codex connection (API-key mode)"
  CODEX_BLOB="$(python3 - <<'EOF'
import base64, datetime, json, os
def jwt(claims):
    part = lambda value: base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip("=")
    return f"{part({'alg': 'none', 'typ': 'JWT'})}.{part(claims)}.e2e"
never = 4102444800
print(json.dumps({
    "auth_mode": "apikey",
    "OPENAI_API_KEY": os.environ["OPENAI_API_KEY"],
    "tokens": {
        "id_token": jwt({"exp": never, "email": "admin@localhost"}),
        "access_token": jwt({"exp": never}),
        "refresh_token": "e2e-none",
        "account_id": None,
    },
    "last_refresh": datetime.datetime.now(datetime.timezone.utc).isoformat(),
}))
EOF
)"
  PGOPTIONS="-c search_path=${SCHEMA}" psql "$DB_URL" -q -v ON_ERROR_STOP=1 -v blob="$CODEX_BLOB" <<'SQL'
INSERT INTO provider_tokens (provider, auth_method, access_token, token_type, connected_at, updated_at, scope)
VALUES ('codex', 'manual', :'blob', 'Bearer', NOW(), NOW(), 'global');
SQL
  unset CODEX_BLOB
fi

# Publishing needs a bucket, and a page's token needs a signing key.
ARTIFACT_PAGES="$(
  set -a
  # shellcheck disable=SC1090
  source "$BASE_ENV"
  if [ -n "${S3_BUCKET_NAME:-}" ] && [ -n "${SESSION_JWT_SECRET:-${TASK_TOKEN_SECRET:-}}" ]; then
    echo 1
  fi
)"

log "Running the suite (profile local)"
# A sandbox's own login outranks MOATLESS_API_KEY in the CLI, so it goes.
env -u IS_SANDBOX -u TASK_ID -u SANDBOX_ID \
  USER_AUTH_TOKEN= \
  HOME="$RUN_DIR" \
  E2E_PROFILE=local \
  E2E_ARTIFACT_PAGES="$ARTIFACT_PAGES" \
  MOATLESS_API_URL="$BASE_URL" \
  FRONTEND_URL="$BASE_URL" \
  ADMIN_USERNAME="$RUN_ADMIN_EMAIL" \
  ADMIN_PASSWORD="$RUN_ADMIN_PASSWORD" \
  MOAT_BIN="${MOAT_BIN:-${BIN_DIR}/moat}" \
  bunx playwright test --config tests/api/playwright.config.ts "$@"
