#!/usr/bin/env bash
# Print the feedback reported per UTC day over the last <days> days, whatever
# its status now, as the page's `meta/trend` record:
#
#   trend.sh [<days>]
#
#   {"days": [{"date": "YYYY-MM-DD", "count": n, "open": n}]}
#
# <days> defaults to 28. `open` is the part of `count` still open now. Every
# day appears, so a quiet day is a 0 rather than a gap the chart would close up.
set -euo pipefail

days="${1:-28}"
page=200
since=$(date -u -d "$((days - 1)) days ago" +%F)

dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
for status in open fixed not_relevant; do
  offset=0
  while :; do
    moat feedback list --status "$status" --since "$since" --json --limit "$page" --offset "$offset" \
      >"$dir/$status-$offset.json"
    total=$(jq '.total' "$dir/$status-$offset.json")
    offset=$((offset + page))
    ((offset < total)) || break
  done
done

jq -s --arg since "$since" --argjson n "$days" '
  (map(.items[]) | unique_by(.feedbackId) | map({d: .createdAt[0:10], open: (.status == "open")})) as $all
  | ($since + "T00:00:00Z" | fromdateiso8601) as $t0
  | {days: [range(0; $n) | ($t0 + . * 86400 | strftime("%Y-%m-%d")) as $d
            | {date: $d,
               count: ([$all[] | select(.d == $d)] | length),
               open: ([$all[] | select(.d == $d and .open)] | length)}]}
' "$dir"/*.json
