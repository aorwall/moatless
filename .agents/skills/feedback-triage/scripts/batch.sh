#!/usr/bin/env bash
# Print the open feedback from the last <days> days as a JSON array, oldest first.
#
#   batch.sh [<days>]
#
# <days> defaults to 7. Older open items are left out on purpose: a run triages
# what is fresh, not the backlog.
set -euo pipefail

days="${1:-7}"
page=200
since=$(date -u -d "$days days ago" +%Y-%m-%dT%H:%M:%SZ)

dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
offset=0
while :; do
  moat feedback list --status open --json --limit "$page" --offset "$offset" --since "$since" \
    >"$dir/$offset.json"
  total=$(jq '.total' "$dir/$offset.json")
  offset=$((offset + page))
  ((offset < total)) || break
done

# Items can shift between pages while new feedback arrives, so dedupe by id.
jq -s '
  map(.items[])
  | unique_by(.feedbackId)
  | sort_by([.createdAt, .feedbackId])
  | map({feedbackId, createdAt, category, source, surface, comment,
         taskId, taskTitle, reportedFrom: .repositoryName, workspaceId})
' "$dir"/*.json
