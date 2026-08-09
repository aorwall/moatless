#!/usr/bin/env bash
#
# Step 7 invariant — bindings does not reach up into adapters.
#
# The dependency Step 7 untangles is bindings->adapters: L1 `bindings` naming
# types from L2 `adapters` is the upward edge of the cycle. Once a bindings file
# stops naming `crate::adapters` it has been cut loose; this check enforces that
# the set of files still naming it only ever shrinks.
#
# The baseline (scripts/bindings-adapters-baseline.txt) lists every file under
# crates/loops/src/bindings/ that still names `crate::adapters` in non-test code. It
# is a ratchet in the shape of the repo's lint baseline (AGENTS.md: "the
# baseline only shrinks"): every slice deletes the entries it clears, a baseline
# entry that no longer names `crate::adapters` is a failure (you cleared it — so
# remove its line), and a bindings file that names `crate::adapters` without
# being listed is a failure (a new up-edge). When the baseline is empty the
# untangle is complete.
#
# There is no permanent whitelist: the end state is zero. adapters depends down
# on bindings (implementing the ports bindings declares), never the reverse.
#
# This is a syntactic check, not a Rust parser, but it tokenizes with the same
# string/comment-blanking pass as check-appstate-composition-root.sh, so a
# mention inside a comment or string literal (a doc comment referencing a
# `crate::adapters` path, a string containing it) is not a naming and does not
# count. Top-level `#[cfg(<test>)] mod NAME { ... }` blocks are dropped too:
# test code naming an adapters type to build a fixture is not a production
# up-edge, the same call the sibling script makes for AppState.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DIR="crates/loops/src/bindings"
BASELINE="scripts/bindings-adapters-baseline.txt"

# ── Tokenize + classify one file ─────────────────────────────────────────────
# Emits 1 if the file names `crate::adapters` in production code, else 0,
# computed on the source after blanking every string literal (normal, raw,
# byte/char) and comment, preserving newlines, and dropping top-level
# `#[cfg(<test>)] mod NAME { ... }` blocks. The blanking pass is identical to
# check-appstate-composition-root.sh.
classify() {
  perl -0777 -ne '
    my $src = $_;
    $src =~ s{
        r(\#*)".*?"\g{1}
      | b?"(?:\\.|[^"\\])*"
      | b?'"'"'(?:\\.|[^'"'"'\\])'"'"'
      | //[^\n]*
      | /\*.*?\*/
    }{ " " . ("\n" x (($& =~ tr/\n//))) }gexs;
    $src =~ s{^[ \t]*\#\[cfg\([^\n]*test[^\n]*\][ \t]*\n[ \t]*(?:pub(?:\([a-z]+\))?[ \t]+)?mod[ \t]+\w+[ \t]*\{.*?\n\}}
             { "\n" x (($&) =~ tr/\n//) }gmse;
    print(($src =~ /\bcrate::adapters\b/) ? "1\n" : "0\n");
  ' "$1"
}

# ── Load the baseline ────────────────────────────────────────────────────────
declare -A IN_BASELINE
if [ -f "$BASELINE" ]; then
  while IFS= read -r line; do
    case "$line" in '' | '#'*) continue ;; esac
    IN_BASELINE["$line"]=1
  done < "$BASELINE"
fi

# ── Test-only module files ───────────────────────────────────────────────────
# A file whose module is declared `#[cfg(<test>)] mod NAME;` in its parent is
# reachable only under cfg(test), so it is test code even without an internal
# cfg attribute. Such a file may legitimately name an adapters type to build a
# fixture; exclude it from the scan, or its naming would enter the baseline and
# could never be cleared. Ported from check-appstate-composition-root.sh.
declare -A TEST_ONLY_FILE
while IFS= read -r parent; do
  dir="$(dirname "$parent")"
  while IFS= read -r name; do
    for cand in "$dir/$name.rs" "$dir/$name/mod.rs"; do
      [ -f "$cand" ] && TEST_ONLY_FILE["$cand"]=1
    done
  done < <(
    awk '
      prev ~ /#\[cfg\((all\()?test/ &&
      $0 ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+[[:space:]]*;/ {
        name = $0
        sub(/.*mod[[:space:]]+/, "", name)
        sub(/[[:space:]]*;.*/, "", name)
        print name
      }
      { prev = $0 }
    ' "$parent"
  )
done < <(grep -rlE '#\[cfg\((all\()?test' "$DIR" --include='*.rs' || true)

# ── Scan ─────────────────────────────────────────────────────────────────────
violations=0
declare -A NAMED_NOW

while IFS= read -r file; do
  [ -n "${TEST_ONLY_FILE[$file]:-}" ] && continue
  read -r names < <(classify "$file")
  [ "$names" = 1 ] || continue
  NAMED_NOW["$file"]=1
  if [ -z "${IN_BASELINE[$file]:-}" ]; then
    echo "::error file=$file::$file names crate::adapters but is not in $BASELINE. bindings must not reach up into adapters; take the port adapters implements, or move the shared type down. See .plans/backend-ports-and-crates.md Step 7."
    violations=$((violations + 1))
  fi
done < <(find "$DIR" -name '*.rs' -type f | sort)

# ── Ratchet: the baseline only shrinks ───────────────────────────────────────
for f in "${!IN_BASELINE[@]}"; do
  if [ ! -f "$f" ]; then
    echo "::error::$f is in $BASELINE but does not exist. Remove the stale entry."
    violations=$((violations + 1))
  elif [ -z "${NAMED_NOW[$f]:-}" ]; then
    echo "::error file=$f::$f is in the baseline but no longer names crate::adapters. Delete its line from $BASELINE — the baseline only shrinks."
    violations=$((violations + 1))
  fi
done

if [ "$violations" -gt 0 ]; then
  echo ""
  echo "::error::bindings reaches up into adapters outside the baseline. See the messages above and .plans/backend-ports-and-crates.md Step 7."
  exit 1
fi

remaining=${#IN_BASELINE[@]}
if [ "$remaining" -gt 0 ]; then
  echo "OK: crate::adapters named under $DIR only in $remaining file(s) still pending in $BASELINE."
else
  echo "OK: no file under $DIR names crate::adapters. The untangle is complete."
fi
