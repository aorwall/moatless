#!/usr/bin/env bash
# A stand-in `moat` for the feedback-triage scripts. Memory entries live in
# $STORE as {version, data} files; `tasks create` refuses with the backend's
# rate-limit 503 as many times as $STORE/fails says, naming $RETRY_AT.
set -euo pipefail

file() { printf '%s/%s.json' "$STORE" "${1//:/%}"; }
echo "$1 $2" >>"$STORE/calls.log"
case "$1 $2" in
  "memory get")
    [[ -f $(file "$3") ]] || { echo 'API error (404): {"error":"Memory entry not found"}' >&2; exit 1; }
    jq --arg k "$3" '{items: [. + {key: $k}]}' "$(file "$3")" ;;
  "memory put")
    key=$3 data=""
    shift 3
    while (($#)); do case $1 in --data) data=$2; shift 2 ;; *) shift ;; esac; done
    (($(printf %s "$data" | wc -c) <= 16384)) || { echo 'API error (400): data exceeds 16384 bytes' >&2; exit 1; }
    version=0
    [[ ! -f $(file "$key") ]] || version=$(jq .version "$(file "$key")")
    jq -n --argjson d "$data" --argjson v "$((version + 1))" '{version: $v, data: $d}' >"$(file "$key")"
    echo '{}' ;;
  "memory list")
    prefix=${4//:/%}
    find "$STORE" -maxdepth 1 -name "$prefix*.json" -printf '%f\n' \
      | jq -R 'rtrimstr(".json") | gsub("%"; ":")' | jq -s '{items: map({key: .})}' ;;
  "memory claim" | "memory release") echo '{}' ;;
  "repos list") echo '[{"name": "moatless", "id": "r1"}]' ;;
  "workspaces list")
    echo '[{"id": "w1", "name": "moatless", "source": "git", "sourceRepositoryId": "r1",
      "sourceKey": "default", "repos": [{"isPrimary": true, "repositoryId": "r1"}]}]' ;;
  "tasks create")
    fails=$(cat "$STORE/fails" 2>/dev/null || echo 0)
    if ((fails > 0)); then
      echo $((fails - 1)) >"$STORE/fails"
      echo "API error (503): {\"error\":\"The github token is rate-limited on github.com, so its access to soaplabs/moatless was not checked. Try again after $RETRY_AT UTC.\"}" >&2
      exit 1
    fi
    while (($#)); do case $1 in --message) printf '%s' "$2" >"$STORE/brief.md"; shift 2 ;; *) shift ;; esac; done
    echo '{"taskId": "11111111-2222-3333-4444-555555555555"}' ;;
  "artifact data") echo '{"items": []}' ;;
  *) echo "stub moat: unhandled $*" >&2; exit 1 ;;
esac
