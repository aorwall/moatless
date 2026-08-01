#!/bin/sh
# Reclaim rustc incremental state from the cargo target directory.
#
# rustc keys an incremental unit per crate *and* build configuration, so each
# distinct clippy/check/test invocation strands another unit — 0.4-1 GB apiece
# for a crate the size of the backend. Cargo has no target-directory GC
# (`cargo clean` is all-or-nothing), and in a sandbox the target directory sits
# on a size-capped volume whose overflow gets the pod evicted.
#
# Dropping `incremental/` keeps every compiled dependency in `deps/`, so only
# the next build per configuration pays a non-incremental compile.
set -eu

TARGET_DIR="${CARGO_TARGET_DIR:-target}"
THRESHOLD_MB="${CARGO_GC_THRESHOLD_MB:-4096}"
INCREMENTAL="$TARGET_DIR/debug/incremental"

if [ ! -d "$INCREMENTAL" ]; then
  echo "No incremental state at $INCREMENTAL — nothing to reclaim." >&2
  exit 0
fi

USED_MB=$(du -sm "$INCREMENTAL" | cut -f1)

if [ "$USED_MB" -lt "$THRESHOLD_MB" ]; then
  echo "Incremental state is ${USED_MB} MB, under the ${THRESHOLD_MB} MB threshold — keeping it." >&2
  echo "Set CARGO_GC_THRESHOLD_MB=0 to reclaim it anyway." >&2
  exit 0
fi

echo "Incremental state is ${USED_MB} MB, at or over the ${THRESHOLD_MB} MB threshold." >&2
rm -rf "$INCREMENTAL"
echo "Removed $INCREMENTAL, reclaiming ${USED_MB} MB." >&2
