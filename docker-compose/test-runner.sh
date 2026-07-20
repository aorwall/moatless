#!/bin/bash
set -euo pipefail

# ── Colors ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ── Defaults ────────────────────────────────────────────────────────────────
TIER="db"
SKIP_BUILD=false
KEEP_RUNNING=false
SINGLE_TEST=""
VERBOSE=false

# ── Argument parsing ───────────────────────────────────────────────────────
usage() {
  cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Build Docker images, start compose, run Rust integration tests, tear down.

Options:
  --tier <db|sandbox|agent|all>   Test tier to run (default: db)
  --test <name>                   Run a single test file (e.g. task_crud)
  --skip-build                    Reuse existing Docker images
  --keep-running                  Don't tear down after tests (for debugging)
  --verbose                       Set RUST_LOG=debug on backend
  -h, --help                      Show this help message

Environment variables (passthrough):
  CLAUDE_CODE_OAUTH_TOKEN   Required for agent tier
  ANTHROPIC_API_KEY         Required for agent tier
  GITHUB_PAT                For repository clone tests
  TASK_TOKEN_SECRET          Auto-generated if unset
  DATA_DIR                   Default: \$HOME/.moatless/data

Examples:
  $(basename "$0")                              # DB-tier tests (fastest)
  $(basename "$0") --tier sandbox               # Sandbox tests (builds sandbox image)
  $(basename "$0") --test task_crud             # Single test file
  $(basename "$0") --skip-build --keep-running  # Reuse images, keep running for debugging
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case $1 in
    --tier)
      TIER="$2"
      shift 2
      ;;
    --test)
      SINGLE_TEST="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --keep-running)
      KEEP_RUNNING=true
      shift
      ;;
    --verbose)
      VERBOSE=true
      shift
      ;;
    -h|--help)
      usage
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      echo "Use --help for usage information"
      exit 1
      ;;
  esac
done

# Validate tier
case $TIER in
  db|sandbox|agent|all) ;;
  *)
    echo -e "${RED}Invalid tier: $TIER (must be db, sandbox, agent, or all)${NC}"
    exit 1
    ;;
esac

# ── Change to project root ─────────────────────────────────────────────────
cd "$(dirname "$0")/.."

# ── Prerequisites ──────────────────────────────────────────────────────────
for cmd in docker cargo; do
  if ! command -v "$cmd" &>/dev/null; then
    echo -e "${RED}Required command not found: $cmd${NC}"
    exit 1
  fi
done

# ── Environment defaults ──────────────────────────────────────────────────
export DATA_DIR="${DATA_DIR:-${HOME}/.moatless/data}"
mkdir -p "$DATA_DIR"

if [ -z "${TASK_TOKEN_SECRET:-}" ]; then
  export TASK_TOKEN_SECRET
  TASK_TOKEN_SECRET=$(openssl rand -hex 32)
  echo -e "${BLUE}Generated TASK_TOKEN_SECRET${NC}"
fi

export APP_IMAGE="aorwall/moatless-vibe-backend:latest"
export SANDBOX_IMAGE="${SANDBOX_IMAGE:-aorwall/moatless-vibe-sandbox-js:latest}"

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_DIR="test-logs"
mkdir -p "$LOG_DIR"

COMPOSE_FILES="-f docker-compose/docker-compose.yml -f docker-compose/docker-compose.ci.yml"

needs_sandbox() {
  [[ "$TIER" == "sandbox" || "$TIER" == "agent" || "$TIER" == "all" ]]
}

# ── Build phase ────────────────────────────────────────────────────────────
if [ "$SKIP_BUILD" = false ]; then
  echo ""
  echo -e "${BLUE}=========================================="
  echo "Building backend Docker image..."
  echo -e "==========================================${NC}"
  docker buildx use desktop-linux 2>/dev/null || true
  docker build -t aorwall/moatless-vibe-backend:latest -f docker/Dockerfile.backend .

  if needs_sandbox; then
    echo ""
    echo -e "${BLUE}=========================================="
    echo "Building sandbox images..."
    echo -e "==========================================${NC}"
    docker/build-sandbox.sh --local
  fi
else
  echo -e "${YELLOW}Skipping build (--skip-build)${NC}"
fi

# ── Cleanup phase ──────────────────────────────────────────────────────────
echo ""
echo -e "${BLUE}=========================================="
echo "Stopping existing moatless containers..."
echo -e "==========================================${NC}"

docker compose $COMPOSE_FILES down --remove-orphans 2>/dev/null || true

SANDBOX_CONTAINERS=$(docker ps -q --filter "label=created-by=moatless" 2>/dev/null || true)
if [ -n "$SANDBOX_CONTAINERS" ]; then
  echo "Stopping sandbox containers..."
  echo "$SANDBOX_CONTAINERS" | xargs docker stop 2>/dev/null || true
fi

SANDBOX_CONTAINERS_STOPPED=$(docker ps -aq --filter "label=created-by=moatless" 2>/dev/null || true)
if [ -n "$SANDBOX_CONTAINERS_STOPPED" ]; then
  echo "Removing stopped sandbox containers..."
  echo "$SANDBOX_CONTAINERS_STOPPED" | xargs docker rm 2>/dev/null || true
fi

echo "Removing Docker volumes for fresh start..."
docker volume rm docker-compose_postgres_data docker-compose_redis_data 2>/dev/null || true

# ── Start phase ────────────────────────────────────────────────────────────
echo ""
echo -e "${BLUE}=========================================="
echo "Starting docker compose (postgres, redis, backend)..."
echo -e "==========================================${NC}"

RUST_LOG_LEVEL="info"
if [ "$VERBOSE" = true ]; then
  RUST_LOG_LEVEL="debug"
fi

RUST_LOG="$RUST_LOG_LEVEL" docker compose $COMPOSE_FILES up -d postgres redis backend

# ── Wait for backend health ────────────────────────────────────────────────
echo ""
echo -n "Waiting for backend health check"
MAX_WAIT=60
ELAPSED=0
while [ $ELAPSED -lt $MAX_WAIT ]; do
  if curl -sf http://localhost:${BACKEND_PORT:-8080}/health &>/dev/null; then
    echo ""
    echo -e "${GREEN}Backend is healthy!${NC}"
    break
  fi
  echo -n "."
  sleep 2
  ELAPSED=$((ELAPSED + 2))
done

if [ $ELAPSED -ge $MAX_WAIT ]; then
  echo ""
  echo -e "${RED}Backend failed to become healthy within ${MAX_WAIT}s${NC}"
  echo "Backend logs:"
  docker compose $COMPOSE_FILES logs backend --tail=50
  docker compose $COMPOSE_FILES down --remove-orphans 2>/dev/null || true
  exit 1
fi

# ── Wait for migrations & reset database ──────────────────────────────────
echo ""
echo -n "Waiting for database migrations"
MIGRATE_WAIT=0
while [ $MIGRATE_WAIT -lt 30 ]; do
  if docker exec moatless-postgres psql -U moatless -d moatless -tAc \
    "SELECT 1 FROM information_schema.tables WHERE table_name='tasks'" 2>/dev/null | grep -q 1; then
    echo ""
    echo -e "${GREEN}Migrations complete${NC}"
    break
  fi
  echo -n "."
  sleep 1
  MIGRATE_WAIT=$((MIGRATE_WAIT + 1))
done

echo -e "${BLUE}Resetting database...${NC}"
docker exec moatless-postgres psql -U moatless -d moatless -c \
  "TRUNCATE task_message_content_blocks, task_message_contexts, task_messages, task_events, task_tags, tasks, repository_plugin_repos, plugin_repos, repositories, trigger_receipts, task_bindings, adapter_event_trigger_sources, schedule_trigger_sources, trigger_rules, triggers, integration_connections, integration_pairing_requests, team_members, teams, sessions, api_keys, provider_tokens, users, commands, file_metadata, tags CASCADE;" \
  2>/dev/null || echo -e "${YELLOW}Database reset skipped${NC}"

# ── Test phase ─────────────────────────────────────────────────────────────
echo ""
echo -e "${BLUE}=========================================="
echo "Running tests (tier: $TIER)"
echo -e "==========================================${NC}"

export MOATLESS_API_URL="http://localhost:${BACKEND_PORT:-8080}"
TEST_EXIT_CODE=0

run_tests() {
  local tier_label="$1"
  shift
  local test_log="$LOG_DIR/test-${tier_label}-${TIMESTAMP}.log"

  echo -e "${BLUE}Running $tier_label tests...${NC}"
  echo "Log: $test_log"

  if "$@" 2>&1 | tee "$test_log"; then
    echo -e "${GREEN}$tier_label tests passed!${NC}"
  else
    echo -e "${RED}$tier_label tests failed!${NC}"
    TEST_EXIT_CODE=1
  fi
  echo ""
}

if [ -n "$SINGLE_TEST" ]; then
  # Single test file. Ignored (env-gated) tests are included — every gated
  # test self-skips when its env gate (SANDBOX_E2E / CLAUDE_SDK_E2E /
  # GITHUB_PROXY_E2E / CHANNEL_CALLBACK_E2E) is not set.
  run_tests "$SINGLE_TEST" cargo test -p moatless-integration-tests --test "$SINGLE_TEST" -- --nocapture --include-ignored
else
  case $TIER in
    db)
      run_tests "db" cargo test -p moatless-integration-tests -- --nocapture
      ;;
    sandbox)
      SANDBOX_E2E=1 run_tests "sandbox" cargo test -p moatless-integration-tests -- --ignored --nocapture
      ;;
    agent)
      CLAUDE_SDK_E2E=1 run_tests "agent" cargo test -p moatless-integration-tests -- --ignored --nocapture
      ;;
    all)
      # Run db tests first (non-ignored)
      run_tests "db" cargo test -p moatless-integration-tests -- --nocapture

      # Then run all ignored tests (sandbox + agent)
      SANDBOX_E2E=1 CLAUDE_SDK_E2E=1 run_tests "ignored" cargo test -p moatless-integration-tests -- --ignored --nocapture
      ;;
  esac
fi

# ── Capture compose logs ───────────────────────────────────────────────────
COMPOSE_LOG="$LOG_DIR/compose-${TIMESTAMP}.log"
echo -e "${BLUE}Capturing compose logs to $COMPOSE_LOG${NC}"
docker compose $COMPOSE_FILES logs --no-color > "$COMPOSE_LOG" 2>&1 || true

# ── Teardown ───────────────────────────────────────────────────────────────
if [ "$KEEP_RUNNING" = true ]; then
  echo ""
  echo -e "${YELLOW}Keeping containers running (--keep-running)${NC}"
  echo "To inspect:"
  echo "  docker compose $COMPOSE_FILES logs backend"
  echo "To tear down:"
  echo "  docker compose $COMPOSE_FILES down --remove-orphans"
else
  echo ""
  echo -e "${BLUE}Tearing down...${NC}"
  docker compose $COMPOSE_FILES down --remove-orphans 2>/dev/null || true
  docker volume rm docker-compose_postgres_data docker-compose_redis_data 2>/dev/null || true
fi

# ── Summary ────────────────────────────────────────────────────────────────
echo ""
if [ $TEST_EXIT_CODE -eq 0 ]; then
  echo -e "${GREEN}=========================================="
  echo "All tests passed!"
  echo -e "==========================================${NC}"
else
  echo -e "${RED}=========================================="
  echo "Some tests failed! Check logs in $LOG_DIR/"
  echo -e "==========================================${NC}"
fi

exit $TEST_EXIT_CODE
