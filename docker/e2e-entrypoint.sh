#!/usr/bin/env bash
# Entrypoint for the Playwright E2E runner (docker/Dockerfile.e2e).
#
# Runs inside the moatless docker-compose network with the repo bind-mounted at
# /workspace. Installs workspace deps (once), then runs the requested suite.
#
#   E2E_SUITE=api   → tests/api  (API-only chains; needs GITHUB_PAT + backend)
#   E2E_SUITE=ui    → tests/ui   (browser; needs the frontend SPA)
#   E2E_SUITE=all   → api then ui
#
# DOCKER_COMPOSE_URL defaults per-suite when unset:
#   api → http://backend:8080   (API served directly by the backend)
#   ui  → http://frontend:80    (SPA + /api proxy served by nginx)
#
# Any extra args are forwarded to `playwright test` (e.g. a spec filter).
set -euo pipefail

cd /workspace

# Playwright's own CI heuristics (workers=1, retries, forbidOnly).
export CI="${CI:-1}"

# node_modules is a named volume (empty on first run, mount point always exists),
# so test for emptiness rather than presence.
if [ -z "$(ls -A node_modules 2>/dev/null)" ]; then
    echo "[e2e] Installing workspace dependencies with bun..."
    bun install --frozen-lockfile
fi

SUITE="${E2E_SUITE:-api}"

run_suite() {
    local suite="$1"; shift
    local config default_url
    case "$suite" in
        api) config="tests/api/playwright.config.ts"; default_url="http://backend:8080" ;;
        ui)  config="tests/ui/playwright.config.ts";  default_url="http://frontend:80" ;;
        *) echo "[e2e] Unknown suite: $suite (use api|ui|all)"; exit 1 ;;
    esac
    echo ""
    echo "=== Running $suite E2E suite ($config) ==="
    DOCKER_COMPOSE_URL="${DOCKER_COMPOSE_URL:-$default_url}" \
        npx playwright test --config "$config" "$@"
}

case "$SUITE" in
    api) run_suite api "$@" ;;
    ui)  run_suite ui "$@" ;;
    all)
        run_suite api "$@"
        run_suite ui "$@"
        ;;
    *) echo "[e2e] Unknown E2E_SUITE: $SUITE (use api|ui|all)"; exit 1 ;;
esac
