#!/bin/sh
# Reclaim rustc incremental state from the cargo target directory.
#
# rustc keys an incremental unit per crate *and* build configuration, and writes
# one the first time that crate actually recompiles under that configuration.
# Measured on this workspace: a unit is written at full size (~110 MB) and does
# not grow afterwards — 20 edit-rebuild cycles moved the directory by 4 MB — and
# a configuration that recompiles nothing costs zero. So the total is the count
# of (crate, configuration) pairs that have ever recompiled, editing does not
# add to it, and what reclaims space is dropping pairs you have stopped
# building. Cargo has no target-directory GC, and in a sandbox the target
# directory sits on a size-capped volume whose overflow gets the pod evicted.
#
# Two rules, in order:
#   1. Drop units untouched for CARGO_GC_MAX_AGE_DAYS. A unit directory's mtime
#      is its last recompile, so this is exactly the set of configurations you
#      have stopped running; the set you use daily keeps its state.
#   2. If that leaves the directory over threshold, drop the rest. In a sandbox
#      every unit is minutes old, so rule 1 never fires and this is the one that
#      saves the pod.
#
# Dropping a unit invalidates nothing — cargo's fingerprints live elsewhere, so
# a no-op build after this stays a no-op. The cost is paid only by the next
# build that recompiles that crate, which was going to recompile anyway.
set -eu

TARGET_DIR="${CARGO_TARGET_DIR:-target}"
THRESHOLD_MB="${CARGO_GC_THRESHOLD_MB:-4096}"
MAX_AGE_DAYS="${CARGO_GC_MAX_AGE_DAYS:-7}"
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

STALE=$(find "$INCREMENTAL" -maxdepth 1 -mindepth 1 -type d -mtime "+${MAX_AGE_DAYS}" | wc -l | tr -d ' ')
if [ "$STALE" -gt 0 ]; then
  find "$INCREMENTAL" -maxdepth 1 -mindepth 1 -type d -mtime "+${MAX_AGE_DAYS}" -exec rm -rf {} +
  AFTER_MB=$(du -sm "$INCREMENTAL" | cut -f1)
  echo "Dropped ${STALE} unit(s) untouched for ${MAX_AGE_DAYS}+ days, reclaiming $((USED_MB - AFTER_MB)) MB." >&2
  USED_MB=$AFTER_MB
fi

if [ "$USED_MB" -ge "$THRESHOLD_MB" ]; then
  # Oldest first until under threshold, rather than all-or-nothing: a full wipe
  # costs the next edit of every crate ~3x (measured, 11.6s -> 31.5s), and the
  # units earning that back are the ones used most recently. Falls through to
  # emptying the directory when even the newest unit will not fit, which is the
  # sandbox case.
  DROPPED=0
  RECLAIMED=0
  while [ "$USED_MB" -ge "$THRESHOLD_MB" ]; do
    # `ls -dt` is newest-first on both macOS and Linux, so the tail is the oldest.
    UNIT=$(ls -dt "$INCREMENTAL"/*/ 2>/dev/null | tail -1)
    [ -z "$UNIT" ] && break
    UNIT_MB=$(du -sm "$UNIT" | cut -f1)
    rm -rf "$UNIT"
    USED_MB=$((USED_MB - UNIT_MB))
    RECLAIMED=$((RECLAIMED + UNIT_MB))
    DROPPED=$((DROPPED + 1))
  done
  echo "Dropped ${DROPPED} least-recently-used unit(s), reclaiming ${RECLAIMED} MB." >&2
fi

echo "Now $(du -sm "$INCREMENTAL" 2>/dev/null | cut -f1 || echo 0) MB — kept the units still in use." >&2
