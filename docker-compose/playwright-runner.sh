#!/bin/bash
set -euo pipefail

# Full-stack Playwright E2E runner. Brings up the docker-compose stack and runs
# the E2E suites in tests/api / tests/ui inside the e2e container, under the
# `docker` profile.
#
#   build backend (+ frontend for ui/all) → start postgres/redis/backend[/frontend]
#   → wait for /health → docker compose run e2e → tear down
#
# The Playwright suites talk to the backend/frontend over the compose network,
# so the browser and gh/docker CLIs all live in the e2e image — nothing but
# Docker is required on the host.
#
# Two host-supplied binaries change how much of that work happens here:
#
#   BACKEND_BINARY — a `moatless-vibe-backend` already compiled for this
#     architecture. With it the backend image is the distroless runtime stage
#     alone (docker/Dockerfile.backend-cross), a few seconds. Without it the
#     image compiles the backend from source in a `--release` build that shares
#     no cache with anything, which is where a CI run spends most of its hour.
#
#   MOAT_BIN_SRC — a compiled `moat`. Every call tests/api makes goes through
#     that CLI (tests/shared/moat.ts), and the e2e image does not ship it: it
#     is a tooling image, and the CLI is version-coupled to the backend under
#     test. Staged into .e2e-bin/, which the e2e service already sees through
#     its repo bind mount.
#
# Both default to building/resolving the old way, so a laptop run needs neither.

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'

SUITE="api"
SKIP_BUILD=false
KEEP_RUNNING=false
EXTRA_ARGS=()

usage() {
  cat <<EOF
Usage: $(basename "$0") [OPTIONS] [-- <playwright args>]

Start the docker-compose stack and run the Playwright E2E suites in a container.

Options:
  --suite <api|ui|all>   Which suite to run (default: api)
  --skip-build           Reuse existing Docker images
  --keep-running         Don't tear down after tests (for debugging)
  -h, --help             Show this help message

Environment variables (passthrough):
  GITHUB_PAT             Required — tests/api validates it via the gh CLI
  ADMIN_PASSWORD         Admin password the suites log in with
                         (default: ci-admin-password-1234, matching docker-compose.ci.yml)
  TASK_TOKEN_SECRET      Auto-generated if unset
  DATA_DIR               Default: \$HOME/.moatless/data
  DOCKER_COMPOSE_URL     Override the target URL (default: per-suite in-network)
  BACKEND_BINARY         Prebuilt moatless-vibe-backend; skips compiling in Docker
  MOAT_BIN_SRC           Prebuilt moat CLI the suites drive the product through
  E2E_WORKERS            Playwright workers (default: 1 under CI, per-suite locally)

Examples:
  $(basename "$0")                                   # API suite (fastest, no browser)
  $(basename "$0") --suite ui                        # Browser suite (builds frontend)
  $(basename "$0") --suite all --skip-build          # Both suites, reuse images
  $(basename "$0") --suite api -- tests/api/specs/task-crud.spec.ts   # single chain
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case $1 in
    --suite) SUITE="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    --keep-running) KEEP_RUNNING=true; shift ;;
    -h|--help) usage ;;
    --) shift; EXTRA_ARGS=("$@"); break ;;
    *) echo -e "${RED}Unknown option: $1${NC}"; echo "Use --help for usage"; exit 1 ;;
  esac
done

case $SUITE in
  api|ui|all) ;;
  *) echo -e "${RED}Invalid suite: $SUITE (must be api, ui, or all)${NC}"; exit 1 ;;
esac

cd "$(dirname "$0")/.."

for cmd in docker; do
  command -v "$cmd" &>/dev/null || { echo -e "${RED}Required command not found: $cmd${NC}"; exit 1; }
done

export DATA_DIR="${DATA_DIR:-${HOME}/.moatless/data}"
mkdir -p "$DATA_DIR"

if [ -z "${TASK_TOKEN_SECRET:-}" ]; then
  export TASK_TOKEN_SECRET
  TASK_TOKEN_SECRET=$(openssl rand -hex 32)
  echo -e "${BLUE}Generated TASK_TOKEN_SECRET${NC}"
fi

export ADMIN_USERNAME="${ADMIN_USERNAME:-admin}"
export ADMIN_PASSWORD="${ADMIN_PASSWORD:-ci-admin-password-1234}"

if [ -z "${GITHUB_PAT:-}" ]; then
  echo -e "${YELLOW}Warning: GITHUB_PAT is unset — tests/api/global-setup.ts will fail fast.${NC}"
fi

export APP_IMAGE="aorwall/moatless-vibe-backend:latest"
# Must match docker/build-app.sh's output image (${REGISTRY_ORG:-aorwall}/moatless-frontend)
# so `--suite ui` runs the freshly built frontend, not a stale published tag.
export FRONTEND_IMAGE="${FRONTEND_IMAGE:-${REGISTRY_ORG:-aorwall}/moatless-frontend:latest}"
export SANDBOX_IMAGE="${SANDBOX_IMAGE:-aorwall/moatless-vibe-sandbox-js:latest}"
export E2E_SUITE="$SUITE"
export E2E_WORKERS="${E2E_WORKERS:-}"

# ── Stage the moat CLI for the e2e container ───────────────────────────────
# .e2e-bin lives under the repo root because that is what the e2e service
# bind-mounts; nothing else reaches inside the container.
export MOAT_BIN="${MOAT_BIN:-moat}"
if [ -n "${MOAT_BIN_SRC:-}" ]; then
  [ -x "$MOAT_BIN_SRC" ] || { echo -e "${RED}MOAT_BIN_SRC is not an executable file: $MOAT_BIN_SRC${NC}"; exit 1; }
  mkdir -p .e2e-bin
  cp "$MOAT_BIN_SRC" .e2e-bin/moat
  chmod +x .e2e-bin/moat
  export MOAT_BIN="/workspace/.e2e-bin/moat"
  echo -e "${BLUE}Staged moat CLI for the e2e container ($MOAT_BIN)${NC}"
fi

COMPOSE_FILES="-f docker-compose/docker-compose.yml -f docker-compose/docker-compose.ci.yml -f docker-compose/docker-compose.e2e.yml"

needs_frontend() { [[ "$SUITE" == "ui" || "$SUITE" == "all" ]]; }

CORE_SERVICES="postgres redis backend"
needs_frontend && CORE_SERVICES="$CORE_SERVICES frontend"

# ── Build phase ────────────────────────────────────────────────────────────
if [ "$SKIP_BUILD" = false ]; then
  if [ -n "${BACKEND_BINARY:-}" ]; then
    [ -x "$BACKEND_BINARY" ] || { echo -e "${RED}BACKEND_BINARY is not an executable file: $BACKEND_BINARY${NC}"; exit 1; }
    echo -e "${BLUE}=== Packaging prebuilt backend binary ===${NC}"
    # A named build context takes a directory, not a file — same handoff
    # ci.yml's build-backend-image job makes.
    BINARY_CTX=$(mktemp -d)
    trap 'rm -rf "$BINARY_CTX"' EXIT
    cp "$BACKEND_BINARY" "$BINARY_CTX/moatless-vibe-backend"
    chmod +x "$BINARY_CTX/moatless-vibe-backend"
    # --build-context is a BuildKit flag; the classic builder ignores the
    # Dockerfile's `--from=binary` and fails on an empty stage.
    DOCKER_BUILDKIT=1 docker build -t "$APP_IMAGE" -f docker/Dockerfile.backend-cross \
      --build-context "binary=$BINARY_CTX" .
  else
    echo -e "${BLUE}=== Building backend image ===${NC}"
    docker build -t "$APP_IMAGE" -f docker/Dockerfile.backend .
  fi
  if needs_frontend; then
    echo -e "${BLUE}=== Building frontend image ===${NC}"
    docker/build-app.sh --local
  fi
  echo -e "${BLUE}=== Building e2e runner image ===${NC}"
  docker compose $COMPOSE_FILES build e2e
else
  echo -e "${YELLOW}Skipping build (--skip-build)${NC}"
fi

# ── Cleanup ────────────────────────────────────────────────────────────────
echo -e "${BLUE}=== Cleaning up previous run ===${NC}"
docker compose $COMPOSE_FILES down --remove-orphans 2>/dev/null || true
docker volume rm docker-compose_postgres_data docker-compose_redis_data 2>/dev/null || true

# ── Start phase ────────────────────────────────────────────────────────────
echo -e "${BLUE}=== Starting stack: $CORE_SERVICES ===${NC}"
# shellcheck disable=SC2086
RUST_LOG="${RUST_LOG:-info}" docker compose $COMPOSE_FILES up -d $CORE_SERVICES

# ── Wait for backend health ────────────────────────────────────────────────
echo -n "Waiting for backend health check"
ELAPSED=0
until curl -sf "http://localhost:${BACKEND_PORT:-8080}/health" &>/dev/null; do
  [ $ELAPSED -ge 60 ] && {
    echo -e "\n${RED}Backend failed to become healthy within 60s${NC}"
    docker compose $COMPOSE_FILES logs backend --tail=50
    docker compose $COMPOSE_FILES down --remove-orphans 2>/dev/null || true
    exit 1
  }
  echo -n "."; sleep 2; ELAPSED=$((ELAPSED + 2))
done
echo -e "\n${GREEN}Backend is healthy!${NC}"

# ── Run Playwright suite ───────────────────────────────────────────────────
echo -e "${BLUE}=== Running Playwright suite: $SUITE ===${NC}"
TEST_EXIT_CODE=0
# shellcheck disable=SC2086
docker compose $COMPOSE_FILES run --rm e2e ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"} || TEST_EXIT_CODE=$?

# ── Teardown ───────────────────────────────────────────────────────────────
if [ "$KEEP_RUNNING" = true ]; then
  echo -e "${YELLOW}Keeping containers running (--keep-running)${NC}"
  echo "  Tear down: docker compose $COMPOSE_FILES down --remove-orphans"
else
  echo -e "${BLUE}=== Tearing down ===${NC}"
  docker compose $COMPOSE_FILES down --remove-orphans 2>/dev/null || true
  docker volume rm docker-compose_postgres_data docker-compose_redis_data 2>/dev/null || true
fi

if [ $TEST_EXIT_CODE -eq 0 ]; then
  echo -e "${GREEN}=== Playwright suite passed! ===${NC}"
else
  echo -e "${RED}=== Playwright suite failed (exit $TEST_EXIT_CODE) ===${NC}"
fi
exit $TEST_EXIT_CODE
