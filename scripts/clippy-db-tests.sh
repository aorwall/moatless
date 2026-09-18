#!/usr/bin/env bash
# Clippy every workspace package that declares a `db-tests` feature.
#
# The set is discovered from cargo metadata, never listed here: a package that
# adds the feature is linted the day it lands, and one that drops it stops
# being named. Nine packages declare it today.
#
# Usage: scripts/clippy-db-tests.sh [--isolated] [-- <extra cargo clippy args>]

set -euo pipefail

ISOLATED=false
EXTRA=()
while [ $# -gt 0 ]; do
  case "$1" in
    --isolated) ISOLATED=true; shift ;;
    --) shift; EXTRA=("$@"); break ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required to discover the db-tests packages" >&2
  exit 1
fi

mapfile -t PACKAGES < <(
  cargo metadata --no-deps --format-version 1 \
    | jq -r '.packages[] | select(.features["db-tests"] != null) | .name' \
    | sort
)

if [ "${#PACKAGES[@]}" -eq 0 ]; then
  echo "cargo metadata found no packages declaring db-tests" >&2
  exit 1
fi

echo "Discovered ${#PACKAGES[@]} db-tests packages: ${PACKAGES[*]}"

# One unified pass links the overlapping dependency graph once; the isolated
# loop costs about four times as much. The loop buys one thing: unified sibling
# features mask a crate that only links when another package enables
# test-support, which a manifest edit causes. CI passes --isolated exactly
# then.
if [ "$ISOLATED" = true ]; then
  for package in "${PACKAGES[@]}"; do
    cargo clippy -p "$package" --all-targets --features db-tests "${EXTRA[@]+"${EXTRA[@]}"}" -- -D warnings
  done
  exit 0
fi

ARGS=()
FEATURES=""
for package in "${PACKAGES[@]}"; do
  ARGS+=(-p "$package")
  if [ -n "$FEATURES" ]; then
    FEATURES="$FEATURES,$package/db-tests"
  else
    FEATURES="$package/db-tests"
  fi
done

cargo clippy --all-targets "${ARGS[@]}" --features "$FEATURES" "${EXTRA[@]+"${EXTRA[@]}"}" -- -D warnings
