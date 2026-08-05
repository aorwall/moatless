#!/usr/bin/env bash
#
# Step 10 invariant — the cross-crate reference census baseline is current.
#
# `crate-census` (tools/crate-census) resolves every path form in the tree —
# crate::/super::/self::/use/re-export chains — to the crate that defines the
# symbol, and lists the illegal edges (a lower band naming a higher one) that
# must go to zero before each Step 10 carve. `scripts/crate-census-baseline.txt`
# is the ratchet: it only shrinks. This check regenerates it and fails if the
# committed file is out of date — either a new illegal edge appeared, or a slice
# cleared one and did not delete its baseline line.
#
# Unlike the other composition-root checks this one builds a small Rust tool
# (syn + walkdir only — not the backend closure), because a regex is exactly the
# instrument that produced the withdrawn count this census replaces.
#
# The tool and this check are deleted at Step 10i: once the crates are real,
# Cargo enforces the bands and an illegal edge stops compiling.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo run -q -p crate-census

if ! git diff --quiet -- scripts/crate-census-baseline.txt; then
  echo "crate-census baseline is stale." >&2
  echo "Regenerate with 'cargo run -p crate-census' and commit scripts/crate-census-baseline.txt." >&2
  git --no-pager diff -- scripts/crate-census-baseline.txt >&2
  exit 1
fi

echo "crate-census baseline is current."
