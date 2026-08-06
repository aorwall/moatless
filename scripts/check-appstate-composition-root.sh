#!/usr/bin/env bash
#
# Step 5 invariant — AppState is named by the composition root and nothing else.
#
# The god object `AppState` (backend/src/state.rs) is decomposed into per-handler
# ports: a handler takes `State(port): State<Arc<dyn Port>>` and a helper takes
# the ports it uses, both extracted from the one router-held `AppState` through
# `axum::extract::FromRef`. Once a module is swept it stops naming `AppState`
# entirely. This check enforces that the name only survives where it must.
#
# Where it must:
#
#   - backend/src/{main,state,compose}.rs — the composition root: `state.rs`
#     defines `AppState` and its `FromRef` impls, `compose.rs`/`main.rs` build and
#     wire it.
#   - every `*/routes.rs` and `backend/src/loops/http/mod.rs` — routing tables,
#     but only while they still register a handler that names `AppState`. A
#     router-assembly fn names `AppState` to hand it to the router:
#     ".plans/backend-ports-and-crates.md" Step 5 — "The root remains one struct,
#     **held by the router**, named by the composition root and by nothing else."
#     A routing table naming `AppState` is that sentence, not an exception to it.
#     The exemption is conditional, not permanent: a router may be generic over
#     its state `S` (`Arc<dyn Port>: FromRef<S>`) once every handler it registers
#     already names its ports, and it then stops naming `AppState` — `tag/routes.rs`
#     is the first. Generic-able iff every registered handler names ports; the
#     router follows the sweep, it cannot lead it (a `State<AppState>` handler
#     forces `AppState: FromRef<S>`, whose only impl is axum's blanket at
#     `S = AppState`, which pins `S` and makes the generic vacuous). So the
#     routing tables are a second ratchet that empties as each router's handlers
#     clear.
#     A routing table must also stay a routing table: this check fails if a
#     whitelisted `routes.rs` grows a handler (an `async fn` or a `State<` param),
#     so the whitelist cannot be quietly grown into a place a handler hides.
#
# Everywhere else, naming `AppState` is a violation — with one temporary escape:
# the sweep baseline (scripts/appstate-composition-root-baseline.txt) lists the
# modules not yet migrated. It is a ratchet in the shape of the repo's lint
# baseline (AGENTS.md: "the baseline only shrinks"): every slice deletes its own
# entries as it clears them, and a baseline entry that no longer names `AppState`
# is itself a failure — you cleared it, so remove it. When the baseline is empty
# the module sweep is done; the routing tables are then the only remaining
# holders, and they clear on the second ratchet above — each router goes generic
# once its handlers are ported — until `AppState` survives in the composition
# root alone.
#
# This is a syntactic check, not a Rust parser, but it tokenizes with the same
# string/comment-blanking pass as check-dao-composition-root.sh, so a mention
# inside a comment or string literal (a doc comment referencing `AppState`, a
# string containing it) is not a naming and does not count.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SRC="backend/src"
BASELINE="scripts/appstate-composition-root-baseline.txt"

# ── Whitelist ────────────────────────────────────────────────────────────────
# The composition root plus the routing tables. The routing-table entry is a
# conditional exemption, not a permanent one: a router names `AppState` only
# while it still registers a handler that does (see the header note). It carries
# a precondition (no handler of its own), checked below.
is_composition_root() {
  case "$1" in
    "$SRC/main.rs" | "$SRC/state.rs" | "$SRC/compose.rs") return 0 ;;
  esac
  return 1
}
is_routing_table() {
  case "$1" in
    */routes.rs | "$SRC/loops/http/mod.rs") return 0 ;;
  esac
  return 1
}

# ── Tokenize + classify one file ─────────────────────────────────────────────
# Emits "<names_appstate> <has_handler>" (each 0/1) computed on the source after
# blanking every string literal (normal, raw, byte/char) and comment, preserving
# newlines, and dropping top-level `#[cfg(<test>)] mod NAME { ... }` blocks (test
# code building an `AppState` for a handler test is not production naming). The
# blanking pass is identical to check-dao-composition-root.sh.
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
    my $names   = ($src =~ /\bAppState\b/)        ? 1 : 0;
    my $handler = ($src =~ /\basync\s+fn\b/ || $src =~ /State</) ? 1 : 0;
    print "$names $handler\n";
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
# cfg attribute (e.g. auth/test_support.rs). Such a file may legitimately build
# an AppState for a handler test and name the type forever — it must be excluded
# from the scan, or its naming would enter the baseline and could never be
# cleared, leaving the ratchet unable to reach empty. Ported verbatim from
# check-dao-composition-root.sh step 1.
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
done < <(grep -rlE '#\[cfg\((all\()?test' "$SRC" --include='*.rs')

# ── Scan ─────────────────────────────────────────────────────────────────────
violations=0
declare -A NAMED_NOW

while IFS= read -r file; do
  [ -n "${TEST_ONLY_FILE[$file]:-}" ] && continue
  read -r names handler < <(classify "$file")

  if is_composition_root "$file"; then
    continue
  fi

  if is_routing_table "$file"; then
    # The routing-table whitelist only grants one thing: permission to NAME
    # AppState (to hold it on the router). A routes.rs that does not name
    # AppState is not using that permission, so its handlers are irrelevant here
    # — e.g. auth/routes.rs and sandbox/routes.rs are routers for AuthState /
    # SandboxApiState and never touch AppState. But a routes.rs that DOES name
    # AppState must stay a pure routing table: a handler in it would be a
    # State<AppState> hiding behind the whitelist, the exact hole the whitelist
    # must not become.
    if [ "$names" = 1 ] && [ "$handler" = 1 ]; then
      echo "::error file=$file::$file names AppState and contains a handler (an 'async fn' or a 'State<' param). A routing table names AppState only to hold it on the router; move the handler into a handlers.rs and migrate it to per-handler ports."
      violations=$((violations + 1))
    fi
    continue
  fi

  [ "$names" = 1 ] || continue
  NAMED_NOW["$file"]=1
  if [ -z "${IN_BASELINE[$file]:-}" ]; then
    echo "::error file=$file::$file names AppState but is not the composition root, not a routing table, and not in the sweep baseline. A handler takes State<Arc<dyn Port>>; a helper takes the ports it uses. See .plans/backend-ports-and-crates.md Step 5."
    violations=$((violations + 1))
  fi
done < <(find "$SRC" -name '*.rs' -type f | sort)

# ── Ratchet: the baseline only shrinks ───────────────────────────────────────
for f in "${!IN_BASELINE[@]}"; do
  if [ ! -f "$f" ]; then
    echo "::error::$f is in $BASELINE but does not exist. Remove the stale entry."
    violations=$((violations + 1))
  elif [ -z "${NAMED_NOW[$f]:-}" ]; then
    echo "::error file=$f::$f is in the sweep baseline but no longer names AppState. Delete its line from $BASELINE — the baseline only shrinks."
    violations=$((violations + 1))
  fi
done

if [ "$violations" -gt 0 ]; then
  echo ""
  echo "::error::AppState is named outside the composition root. See the messages above and .plans/backend-ports-and-crates.md Step 5."
  exit 1
fi

remaining=${#IN_BASELINE[@]}
if [ "$remaining" -gt 0 ]; then
  echo "OK: AppState named only in the composition root, the routing tables, and $remaining module(s) still pending in $BASELINE."
else
  echo "OK: AppState named only in the composition root and the routing tables. The sweep is complete."
fi
