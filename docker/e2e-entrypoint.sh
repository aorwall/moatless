#!/usr/bin/env bash
# Entrypoint for the Playwright E2E runner (docker/Dockerfile.e2e).
#
# Runs inside the moatless docker-compose network with the repo bind-mounted at
# /workspace. Installs workspace deps (once), then runs the requested suite.
#
#   E2E_SUITE=api   → tests/api  (API-only chains; needs GITHUB_PAT + backend)
#
# DOCKER_COMPOSE_URL defaults to http://backend:8080 when unset.
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
        *) echo "[e2e] Unknown suite: $suite (use api)"; exit 1 ;;
    esac
    echo ""
    echo "=== Running $suite E2E suite ($config) ==="
    DOCKER_COMPOSE_URL="${DOCKER_COMPOSE_URL:-$default_url}" \
        npx playwright test --config "$config" "$@"
}

case "$SUITE" in
    api) run_suite api "$@" ;;
    *) echo "[e2e] Unknown E2E_SUITE: $SUITE (use api)"; exit 1 ;;
esac
