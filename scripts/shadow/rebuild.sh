#!/usr/bin/env bash
# rebuild.sh — Trigger a shadow-backend rebuild + restart.
#
# Run this after editing backend/ (or a shared crate). It compiles in your
# terminal for fast feedback, then bumps the trigger file that up.sh watches so
# the running backend restarts with the new binary. Frontend changes need no
# action — the bun dev server hot-reloads them already.
set -euo pipefail

cd "$(dirname "$0")/../.."
TRIGGER="${SHADOW_TRIGGER:-/opt/moatless/data/shadow/rebuild.trigger}"
mkdir -p "$(dirname "$TRIGGER")"

echo "==> Building moatless-vibe-backend (release)"
cargo build --release -p moatless-vibe-backend

# Bump the trigger's mtime so up.sh restarts the process.
touch "$TRIGGER"
echo "==> Rebuild done; restart triggered"
