#!/usr/bin/env bash
# Print the id of every open feedback item, whatever its age, as a JSON array:
#
#   open-ids.sh
#
# Closing a cluster closes only the ids in its `allMembers` that this lists, so
# an item someone already closed keeps its status and note.
set -euo pipefail

page=200
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
offset=0
while :; do
  moat feedback list --status open --json --limit "$page" --offset "$offset" >"$dir/$offset.json"
  total=$(jq '.total' "$dir/$offset.json")
  offset=$((offset + page))
  ((offset < total)) || break
done
jq -s 'map(.items[].feedbackId) | unique' "$dir"/*.json
