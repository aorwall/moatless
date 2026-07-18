#!/usr/bin/env bash
# up.sh — Run the shadow backend inside a moatless sandbox.
#
# This is the native equivalent of `mirrord exec -- cargo run`: because the
# sandbox is already a pod in the dev cluster, outgoing traffic + DNS are free,
# so we only need to (1) sync the real backend env and (2) run the binary.
#
# The frontend dev server (bun run dev, port 3000) already proxies `/api` to
# http://localhost:8080 (see apps/frontend/vite.config.ts), and sidecar servers
# share the pod's localhost — so nothing on the frontend side needs changing.
#
# Rebuild loop: edit code, then run `scripts/shadow/rebuild.sh`. This process
# watches a trigger file on the shared workspace and restarts the binary.
set -euo pipefail

cd "$(dirname "$0")/../.."
REPO_ROOT="$(pwd)"

BIN="${REPO_ROOT}/target/release/moatless-vibe-backend"
ENV_FILE="${SHADOW_ENV_FILE:-/opt/moatless/data/shadow/backend.env}"
TRIGGER="${SHADOW_TRIGGER:-/opt/moatless/data/shadow/rebuild.trigger}"
mkdir -p "$(dirname "$TRIGGER")"

echo "==> Syncing backend env from the cluster"
scripts/shadow/sync-env.sh "$ENV_FILE"

build() {
  echo "==> Building moatless-vibe-backend (release, incremental)"
  cargo build --release -p moatless-vibe-backend
}

build
[ -f "$TRIGGER" ] || : > "$TRIGGER"
LAST="$(stat -c %Y "$TRIGGER" 2>/dev/null || echo 0)"

# Run the binary; restart it when the trigger file changes (a rebuild) or when
# it exits on its own.
while true; do
  set -a; . "$ENV_FILE"; set +a
  echo "==> Starting shadow backend on :$PORT (grpc :$GRPC_PORT)"
  "$BIN" & PID=$!

  while kill -0 "$PID" 2>/dev/null; do
    NOW="$(stat -c %Y "$TRIGGER" 2>/dev/null || echo 0)"
    if [ "$NOW" != "$LAST" ]; then
      LAST="$NOW"
      echo "==> Rebuild trigger detected — restarting"
      kill "$PID" 2>/dev/null || true
      wait "$PID" 2>/dev/null || true
      build || { echo "!! build failed; waiting for next trigger"; }
      break
    fi
    sleep 2
  done
  # If the process died without a trigger, pause briefly so we don't hot-loop.
  kill -0 "$PID" 2>/dev/null || sleep 2
done
