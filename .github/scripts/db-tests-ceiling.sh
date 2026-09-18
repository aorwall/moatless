#!/usr/bin/env bash
# Hold the db-tests failure count to a per-binary ceiling.
#
# usage: db-tests-ceiling.sh <junit-dir> <tier-exit-status>
#
# Exits 0 when every reporting binary is at or under its entry below, 1 when one
# rises above it, when a binary reports failures with no entry at all, when
# nextest reports an `errors` the ceiling does not cover, or when the tier went
# red without writing a readable report — including a report that parses to
# nothing, which each root's own aggregate is read to catch.
set -euo pipefail

REPORT_DIR=${1:?usage: db-tests-ceiling.sh <junit-dir> <tier-exit-status>}
TIER_STATUS=${2:?usage: db-tests-ceiling.sh <junit-dir> <tier-exit-status>}

# Keyed by binary, not one total, because the run is the changed-file package
# closure: one number would measure a subset excluding `moatless-backend`
# against the whole codebase's figure. Only binaries that reported are compared,
# so the same table holds a full push to `main`. Mirrors the enumeration in
# docs/deferred-changes.md; move both together, downward, and delete both at 0.
CEILINGS="
moatless-backend::integration_surface 49
moatless-backend::integration_adapters 8
moatless-backend::integration_data 1
moatless-loops 8
moatless-tasks 5
moatless-sandbox-control 1
"

shopt -s nullglob
reports=("$REPORT_DIR"/junit*.xml)
shopt -u nullglob

for report in "${reports[@]}"; do
  if ! grep -q '<testsuites' "$report"; then
    echo "::error::$report has no <testsuites> root element, so the db-tests ceiling cannot be read. nextest's JUnit shape changed — fix .github/scripts/db-tests-ceiling.sh before trusting this gate."
    exit 1
  fi
done

if [ ${#reports[@]} -eq 0 ]; then
  # A red run that wrote no report never reached the reporter, so a count of
  # zero here is not a result the ceiling may absorb.
  if [ "$TIER_STATUS" -ne 0 ]; then
    echo "::error::The db-tests tier failed before writing a JUnit report — read the log above rather than the ceiling."
    exit 1
  fi
  echo "No db-tests JUnit reports in $REPORT_DIR, and the tier exited 0."
  exit 0
fi

# `<testsuite>` per binary for the comparison, and the root `<testsuites>` as a
# cross-check only: it carries the same attribute names, so summing the two
# together would count everything twice.
summary=$(
  # `|| true` because `--no-tests=warn` lets a package declare the feature and
  # gate no test behind it: that report has a root and no `<testsuite>`, and a
  # `grep` finding nothing would abort the script under `pipefail`.
  { grep -hoE '<testsuites? [^>]*>' "${reports[@]}" || true; } | awk -v ceilings="$CEILINGS" '
    function attr(element, key,   start, len) {
      if (!match(element, " " key "=\"[^\"]*\"")) return ""
      start = RSTART + length(key) + 3
      len = RLENGTH - length(key) - 4
      return substr(element, start, len)
    }
    BEGIN {
      rows = split(ceilings, lines, "\n")
      for (i = 1; i <= rows; i++) {
        if (split(lines[i], field, " ") == 2) allowed[field[1]] = field[2]
      }
    }
    /^<testsuites / {
      root_failures += attr($0, "failures") + 0
      root_errors += attr($0, "errors") + 0
      next
    }
    {
      name = attr($0, "name")
      if (name == "") next
      failures[name] += attr($0, "failures") + 0
      errors[name] += attr($0, "errors") + 0
      seen[name] = 1
    }
    END {
      for (name in seen) {
        printf "SUITE\t%s\t%d\t%d\t%d\n", name, failures[name], errors[name], allowed[name] + 0
        total_failures += failures[name]
        total_errors += errors[name]
        total_allowed += allowed[name] + 0
      }
      printf "TOTAL\t%d\t%d\t%d\n", total_failures + 0, total_errors + 0, total_allowed + 0
      printf "ROOT\t%d\t%d\n", root_failures + 0, root_errors + 0
    }
  '
)

printf '%s\n' "$summary" | sed -n 's/^SUITE\t\(.*\)\t\([0-9]*\)\t\([0-9]*\)\t\([0-9]*\)$/  \1: \2 failing, \3 errored, ceiling \4/p' | sort

read -r total_failures total_errors total_allowed <<<"$(printf '%s\n' "$summary" | sed -n 's/^TOTAL\t//p' | tr '\t' ' ')"
read -r root_failures root_errors <<<"$(printf '%s\n' "$summary" | sed -n 's/^ROOT\t//p' | tr '\t' ' ')"

# The aggregate the root already carries, against the children this compares:
# a root proven to parse says nothing about the elements one level down, and a
# `<testsuite` the match stops recognising reads as every binary being green.
# Agreement is the only thing in reach that sees a partial parse.
if [ "$total_failures" -ne "$root_failures" ] || [ "$total_errors" -ne "$root_errors" ]; then
  echo "::error::db-tests JUnit is internally inconsistent: the roots report ${root_failures} failing and ${root_errors} errored, the per-binary elements sum to ${total_failures} and ${total_errors}. nextest's JUnit shape changed — fix .github/scripts/db-tests-ceiling.sh before trusting this gate."
  exit 1
fi

# A red tier whose reports total zero contradicts itself, whatever the reports
# parsed to. The guard above reads the files; this one reads the tier.
if [ "$TIER_STATUS" -ne 0 ] && [ "$((total_failures + total_errors))" -eq 0 ]; then
  echo "::error::The db-tests tier exited nonzero but its reports carry no failure — read the log above rather than the ceiling."
  exit 1
fi

status=0

# Never absorbed: nextest reports a test it could not run — a binary that
# failed to execute, or one `slow-timeout`'s `terminate-after` killed — under
# `errors`, and the table above counts assertions.
if [ "$total_errors" -ne 0 ]; then
  echo "::error::db-tests reported ${total_errors} errored test(s), which the ceiling does not cover — a test that could not run or was terminated. Read the log above."
  status=1
fi

over=$(printf '%s\n' "$summary" | awk -F'\t' '$1 == "SUITE" && $3 > $5 { printf "%s (%d failing, ceiling %d) ", $2, $3, $5 }')
if [ -n "$over" ]; then
  echo "::error::db-tests failures rose above their ceiling: ${over}— fix the new failure, or lower the count in .github/scripts/db-tests-ceiling.sh and docs/deferred-changes.md together if this PR repaired fixtures."
  status=1
fi

if [ "$status" -ne 0 ]; then
  exit 1
fi

if [ "$total_failures" -ne 0 ]; then
  echo "::warning::${total_failures} db-tests failing across the binaries that reported, each at or under its ceiling (${total_allowed} allowed) — see docs/deferred-changes.md, 'The db-tests fixtures'."
else
  echo "Every db-tests binary that reported is green."
fi
