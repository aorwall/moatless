#!/usr/bin/env bash
#
# Step 10 invariant — the cross-crate reference census baseline is current.
#
# `crate-census` (tools/crate-census) resolves every path form in the tree —
# crate::/super::/self::/use/re-export chains — to the crate that defines the
# symbol, and lists the illegal edges (a lower band naming a higher one) that
# must go to zero before each Step 10 carve. `scripts/crate-census-baseline.txt`
# is the ratchet: it only shrinks. Its `# Illegal edge count:` line is an
# enforced CEILING, not a running tally — regeneration lowers it to the current
# count via min() but never raises it, so a branch that adds an illegal edge
# cannot make this gate green by regenerating; the count stays pinned below the
# edge list and `--check` fails until the edge is removed (or the ceiling is
# raised by hand, a deliberate reviewed decision). `--check` regenerates the
# census in memory and compares it against the files ON DISK (the working tree),
# failing if either is out of date — a new illegal edge appeared, or a slice
# cleared one and did not delete its baseline line — or if the current count
# exceeds the committed ceiling. It compares against the working tree,
# NOT git (which it never invokes), so a regenerated-but-uncommitted baseline or
# report makes this pass locally while CI — which checks out the committed tree
# — fails; regenerate and commit BOTH files and run this against a clean tree.
# It writes nothing and runs no git, so the CI job needs neither a writable tree
# nor a checkout marked safe for `git diff`.
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

cargo run -q -p crate-census -- --check

echo "crate-census baseline is current."
