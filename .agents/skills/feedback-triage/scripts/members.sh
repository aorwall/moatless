#!/usr/bin/env bash
# Read or grow the full member set of a cluster: every feedback id it ever held.
#
#   members.sh read <slug>          # prints the ids as one JSON array
#   members.sh add <slug> < ids.json  # adds a JSON array of ids, prints the total
#
# The set lives in `triage:members:<slug>:<n>` entries of at most `CHUNK` ids
# each, since a cluster entry with hundreds of ids passes the 16 KiB
# `MAX_BODY_BYTES`. `read` also takes the ids in the entry's own `allMembers`,
# `members` and any `allMembersKey` entry, which entries written before the
# chunks hold. `add` rewrites every chunk, restarting their 28-day expiry with
# the entry's: write it each time the cluster entry is written.
set -euo pipefail

scope="${SCOPE:-loop}"
chunk="${CHUNK:-250}"
cmd="${1:?usage: members.sh read|add <slug>}"
slug="${2:?usage: members.sh read|add <slug>}"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT

# A missing key is a 404: no ids there. Any other failure stops the script,
# since `add` would otherwise rewrite a chunk without the ids it failed to read.
data() {
  if moat memory get "$1" --scope "$scope" --json >"$dir/get.json" 2>"$dir/get.err"; then
    jq '.items[0].data // null' "$dir/get.json"
  elif grep -q '404' "$dir/get.err"; then
    echo null
  else
    cat "$dir/get.err" >&2
    exit 1
  fi
}

read_ids() {
  data "triage:cluster:$slug" >"$dir/entry.json"
  jq '[.allMembers // [], .members // []] | add' "$dir/entry.json" >"$dir/ids-entry.json"
  extra=$(jq -r '.allMembersKey // empty' "$dir/entry.json")
  if [[ -n "$extra" ]]; then
    data "$extra" | jq '.allMembers // .ids // . | if type == "array" then . else [] end'
  else
    echo '[]'
  fi >"$dir/ids-extra.json"
  moat memory list --prefix "triage:members:$slug:" --scope "$scope" --limit 500 --json \
    | jq -r '.items[].key' >"$dir/keys.txt"
  i=0
  while read -r k; do
    data "$k" | jq '.ids // []' >"$dir/ids-chunk-$i.json"
    i=$((i + 1))
  done <"$dir/keys.txt"
  # Drop repeats, keeping the first of each id.
  jq -s 'add | reduce .[] as $id ({seen: {}, ids: []}; if .seen[$id] then . else .seen[$id] = true | .ids += [$id] end) | .ids' \
    "$dir"/ids-*.json
}

case "$cmd" in
  read) read_ids ;;
  add)
    jq 'if type == "array" then . else error("ids must be a JSON array") end' >"$dir/new.json"
    read_ids >"$dir/old.json"
    jq -s '.[0] + .[1] | reduce .[] as $id ({seen: {}, ids: []}; if .seen[$id] then . else .seen[$id] = true | .ids += [$id] end) | .ids' \
      "$dir/old.json" "$dir/new.json" >"$dir/all.json"
    total=$(jq length "$dir/all.json")
    for ((n = 0; n * chunk < total; n++)); do
      body=$(jq -c --argjson n "$n" --argjson c "$chunk" '{ids: .[$n * $c:($n + 1) * $c]}' "$dir/all.json")
      if (($(printf %s "$body" | wc -c) > 16384)); then
        echo "members.sh: chunk $n of $slug passes 16 KiB; lower CHUNK" >&2
        exit 1
      fi
      moat memory put "triage:members:$slug:$n" --scope "$scope" \
        --summary "Members of $slug, part $n" --data "$body" >/dev/null
    done
    echo "$total"
    ;;
  *) echo "members.sh: unknown command $cmd" >&2; exit 1 ;;
esac
