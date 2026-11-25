#!/usr/bin/env bash
# up.sh — Run the shadow backend inside a moatless sandbox.
#
# This is the native equivalent of `mirrord exec -- cargo run`: because the
# sandbox is already a pod in the dev cluster, outgoing traffic + DNS are free,
# so we only need to (1) sync the real backend env and (2) run the binary.
#
# Rebuild loop: edit code, then run `scripts/shadow/rebuild.sh`. This process
# watches a trigger file on the shared workspace and restarts the binary.
set -euo pipefail

cd "$(dirname "$0")/../.."
REPO_ROOT="$(pwd)"

# Cargo target dir. A precompiled sandbox image bakes a warm target dir at a
# path OUTSIDE the workspace volume (`/opt/moatless` is a mount, so a cache under
# it is shadowed on every provision). When CARGO_TARGET_DIR points at that baked
# dir, `cargo build` reuses the compiled dependency artifacts and only recompiles
# changed workspace crates. If the baked binary matches this checkout exactly,
# we skip that initial rebuild entirely and start the backend immediately. Falls
# back to the in-tree `target/` when unset (plain sandbox image).
TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"

# Cargo profile to build the shadow backend with. `release` (the default, and
# what the dev image bakes) takes ~4m34s to relink after a one-file backend
# edit; `dev` takes ~21s and shares its artifacts with `cargo test`/`cargo
# clippy`, so an agent iterating on the backend pays for one build instead of
# two. `ci` (release opts, no LTO) sits in between. See README.md.
PROFILE="${SHADOW_CARGO_PROFILE:-release}"
# `dev` is the one profile whose artifacts do not land in target/<profile>/.
if [ "$PROFILE" = "dev" ]; then PROFILE_DIR="debug"; else PROFILE_DIR="$PROFILE"; fi
BIN="${TARGET_DIR}/${PROFILE_DIR}/moatless-backend"
# The dev image bakes a *release* binary and stamps its commit at the target
# root. On any other profile there is no baked binary, so the check below must
# never match — an empty stamp path makes it fail fast.
if [ "$PROFILE" = "release" ]; then
  SOURCE_STAMP="${TARGET_DIR}/.moatless-source-commit"
else
  SOURCE_STAMP=""
fi
ENV_FILE="${SHADOW_ENV_FILE:-${RUNTIME_DIR:-/run/moatless}/shadow/backend.env}"
TRIGGER="${SHADOW_TRIGGER:-/opt/moatless/data/shadow/rebuild.trigger}"
BAKED_COMMIT=""
mkdir -p "$(dirname "$TRIGGER")"

echo "==> Syncing backend env from the cluster"
scripts/shadow/sync-env.sh "$ENV_FILE"

build() {
  if [ -d "${TARGET_DIR}/${PROFILE_DIR}/deps" ]; then
    echo "==> Building moatless-backend (${PROFILE}, warm cache: ${TARGET_DIR})"
  else
    echo "==> Building moatless-backend (${PROFILE}, COLD - no baked cache at ${TARGET_DIR})"
  fi
  cargo build --profile "$PROFILE" -p moatless-backend
}

baked_binary_matches_checkout() {
  [ -n "$SOURCE_STAMP" ] || return 1
  [ -x "$BIN" ] || return 1
  [ -f "$SOURCE_STAMP" ] || return 1

  local current_commit
  BAKED_COMMIT="$(cat "$SOURCE_STAMP" 2>/dev/null || true)"
  current_commit="$(git rev-parse HEAD 2>/dev/null || true)"
  [ -n "$BAKED_COMMIT" ] && [ "$BAKED_COMMIT" != "unknown" ] || return 1
  [ -n "$current_commit" ] && [ "$BAKED_COMMIT" = "$current_commit" ] || return 1

  # The baked binary is only safe before Rust-relevant local edits. After edits,
  # rebuild.sh touches the trigger and the loop below always runs cargo build.
  [ -z "$(git status --porcelain -- Cargo.toml Cargo.lock rust-toolchain.toml .cargo crates/server crates 2>/dev/null)" ]
}

if baked_binary_matches_checkout; then
  echo "==> Using baked moatless-backend from ${TARGET_DIR} (source ${BAKED_COMMIT})"
else
  build
fi

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
