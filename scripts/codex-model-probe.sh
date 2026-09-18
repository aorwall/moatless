#!/usr/bin/env bash
# Print the model catalog baked into one or two `@openai/codex` builds.
#
#   scripts/codex-model-probe.sh 0.154.0            # one catalog
#   scripts/codex-model-probe.sh 0.154.0 0.157.0    # both, plus a diff
#
# Each line is `id<TAB>displayName<TAB>hidden<TAB>isDefault<TAB>efforts`, in the
# order `model/list` returns them, which is the order the catalogs in
# `crates/tasks/src/agent/catalog.rs` and `crates/t3code/src/agent/catalog.rs`
# are written in.
#
# CODEX_HOME is cleared per probe, and that is the whole point of this script.
# A sandbox pod points CODEX_HOME at a rendered config that routes Codex through
# the Moatless proxy, and `model/list` answers with *that endpoint's* catalog:
# probing this pod's own CODEX_HOME returns the proxy's Claude models and calls
# `gpt-5.6-sol` the default. Writing those into the Codex catalog would hand
# every Codex task a model id Codex cannot run.

set -euo pipefail

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: $0 <codex-version> [<codex-version>]" >&2
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Fetch one version's catalog into $2, normalized to TSV.
probe() {
  local version=$1 out=$2
  local prefix="$WORK/$version"

  mkdir -p "$prefix"
  if ! npm i --prefix "$prefix" "@openai/codex@$version" --no-audit --no-fund >"$WORK/$version.npm.log" 2>&1; then
    echo "npm could not install @openai/codex@$version:" >&2
    tail -5 "$WORK/$version.npm.log" >&2
    exit 1
  fi

  local bin="$prefix/node_modules/.bin/codex"
  if [ ! -x "$bin" ]; then
    echo "codex@$version did not install a runnable binary at $bin" >&2
    exit 1
  fi

  local raw="$WORK/$version.json"
  # A fresh, empty CODEX_HOME: no auth.json, no config.toml, no routed provider.
  {
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"probe","title":"probe","version":"0.0.0"}}}'
    sleep 3
    printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"model/list","params":{}}'
    sleep 6
  } | CODEX_HOME="$(mktemp -d -p "$WORK")" timeout 25 "$bin" app-server 2>/dev/null \
    | grep '"id":2' > "$raw" || true

  if [ ! -s "$raw" ]; then
    echo "codex@$version returned no model/list response" >&2
    exit 1
  fi

  jq -r '.result.data[]
         | [ .id,
             .displayName,
             (if (.hidden // false) then "hidden" else "-" end),
             (if (.isDefault // false) then "DEFAULT" else "-" end),
             ([.supportedReasoningEfforts[]?.reasoningEffort] | join(",")) ]
         | @tsv' "$raw" > "$out"
}

probe "$1" "$WORK/a.tsv"
echo "== codex@$1"
cat "$WORK/a.tsv"

if [ $# -eq 2 ]; then
  probe "$2" "$WORK/b.tsv"
  echo
  echo "== codex@$2"
  cat "$WORK/b.tsv"
  echo
  echo "== $1 -> $2"
  if diff -u --label "codex@$1" "$WORK/a.tsv" --label "codex@$2" "$WORK/b.tsv"; then
    echo "(no catalog change)"
  fi
fi
