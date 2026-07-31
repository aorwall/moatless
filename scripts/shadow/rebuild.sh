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

# Must match the profile up.sh runs from, or this rebuilds one binary while the
# backend keeps running the other. Defaults to `release`; `dev` is ~13x faster.
PROFILE="${SHADOW_CARGO_PROFILE:-release}"

echo "==> Building moatless-vibe-backend (${PROFILE})"
cargo build --profile "$PROFILE" -p moatless-vibe-backend

# Bump the trigger's mtime so up.sh restarts the process.
touch "$TRIGGER"
echo "==> Rebuild done; restart triggered"
