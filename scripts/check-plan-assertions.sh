#!/usr/bin/env bash
#
# Plan-claim invariant — a spec in .plans/ does not outlive the code it asserts.
#
# The census ratchets edge COUNTS and the sibling checks ratchet file SETS, but
# nothing re-validated the prose CLAIMS a plan makes about the source. That gap
# is not hypothetical. This program's plan asserted step 8h was blocked because
# `backend/src/setup/github.rs` reached `credentials` (band 4) and `sandbox`
# (band 6). Two merged PRs, neither aimed at 8h, moved both symbols elsewhere.
# The claim stayed in the plan and would have sent the next round of work at the
# wrong target; it was caught only because someone re-read the source instead of
# trusting the document.
#
# So: a plan may carry machine-checkable assertions beside the claims that gate
# work, and this check fails when one stops being true. It is deliberately
# opt-in — a plan asserts what is load-bearing, not everything it mentions. An
# assertion that fails is not necessarily a bug in the code; it usually means
# the plan needs rewriting, which is exactly the signal that was missing.
#
# Syntax, anywhere in a .plans/**/*.md file (an HTML comment, so it renders as
# nothing), placed immediately after the sentence it backs:
#
#   <!-- plan-assert: exists   <path> -->
#   <!-- plan-assert: contains <path> <literal string> -->
#   <!-- plan-assert: absent   <path> <literal string> -->
#
# `contains`/`absent` are fixed-string, whole-file, comments and strings
# included — this is a staleness tripwire, not a parser, and the sibling checks
# already own tokenized band analysis. The needle is everything after the path,
# so it may contain spaces. Failures report the .plans file and line of the
# assertion, because the document is what needs the edit.
#
# An assertion must occupy its whole line. That is what lets a plan document the
# mechanism in prose — quoting the marker inline, inside backticks — without the
# sentence being parsed as an assertion. This check found that hole in itself on
# its first run against its own documentation.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail=0
checked=0

# Collect assertions as: <plan-file>:<line>:<kind>:<path>:<needle>
while IFS= read -r hit; do
  plan_file=${hit%%:*}
  rest=${hit#*:}
  plan_line=${rest%%:*}
  body=${rest#*:}

  # Strip the comment wrapper and the marker, then split kind / path / needle.
  # `%-->*` is the shortest suffix, i.e. the LAST `-->` on the line, so a needle
  # may itself contain `-->`.
  body=${body#*<!-- plan-assert:}
  body=${body%-->*}

  # Split with a regex rather than `set -- $body`: word splitting would collapse
  # runs of whitespace inside a needle and pathname expansion would rewrite one
  # containing `*`, `?` or `[`, both of which silently change the fixed string
  # this check promises to look for.
  kind=; path=; needle=
  if [[ $body =~ ^[[:space:]]*([A-Za-z]+)([[:space:]]+([^[:space:]]+)([[:space:]]+(.*))?)?[[:space:]]*$ ]]; then
    kind=${BASH_REMATCH[1]}
    path=${BASH_REMATCH[3]}
    needle=${BASH_REMATCH[5]}
    # The regex's trailing `[[:space:]]*` cannot claim anything while `(.*)` is
    # greedy, so trim the needle's trailing whitespace here instead. A needle
    # therefore cannot end in a space — no literal worth asserting does.
    needle=${needle%"${needle##*[![:space:]]}"}
  fi

  checked=$((checked + 1))
  loc="$plan_file:$plan_line"

  # A `contains`/`absent` with no needle greps for the empty string, which
  # matches every file — it would pass while asserting nothing at all.
  if [[ "$kind" == "contains" || "$kind" == "absent" ]] && [[ -z "$needle" ]]; then
    echo "FAIL $loc — '$kind' needs a path and a non-empty literal to look for"
    fail=1
    continue
  fi

  case "$kind" in
    exists)
      if [[ -z "$path" ]]; then
        echo "FAIL $loc — 'exists' needs a path"
        fail=1
      elif [[ ! -e "$path" ]]; then
        echo "FAIL $loc — asserts '$path' exists; it does not"
        fail=1
      fi
      ;;
    contains)
      if [[ ! -f "$path" ]]; then
        echo "FAIL $loc — asserts '$path' contains \"$needle\"; the file does not exist"
        fail=1
      elif ! grep -qF -- "$needle" "$path"; then
        echo "FAIL $loc — asserts '$path' contains \"$needle\"; it does not"
        fail=1
      fi
      ;;
    absent)
      if [[ ! -f "$path" ]]; then
        echo "FAIL $loc — asserts '$path' does not contain \"$needle\"; the file does not exist"
        fail=1
      elif grep -qF -- "$needle" "$path"; then
        echo "FAIL $loc — asserts '$path' does NOT contain \"$needle\"; it does"
        fail=1
      fi
      ;;
    *)
      echo "FAIL $loc — unknown assertion kind '$kind' (want exists|contains|absent)"
      fail=1
      ;;
  esac
done < <(grep -rnE '^[[:space:]]*<!-- plan-assert:.*-->[[:space:]]*$' .plans --include='*.md' 2>/dev/null || true)

if [[ "$checked" -eq 0 ]]; then
  echo "OK — no plan assertions declared."
  exit 0
fi

if [[ "$fail" -ne 0 ]]; then
  cat <<'EOF'

A plan asserts something about the source that is no longer true.

This is usually the plan being stale, not the code being wrong: a claim that
gated work has quietly stopped holding. Re-read the section that carries the
failing assertion and rewrite it, then update or delete the assertion. Do not
"fix" the code to satisfy a document.
EOF
  exit 1
fi

echo "OK — $checked plan assertion(s) hold."
