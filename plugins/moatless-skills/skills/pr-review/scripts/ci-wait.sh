#!/usr/bin/env bash
# Wait until every check on a pull request's head has finished.
# Usage: ci-wait.sh OWNER/REPO NUMBER REVIEWED_SHA [INTERVAL_SECONDS]
# Prints one line and exits 0 when all passed or were skipped, 1 when one
# failed or was cancelled, 3 when the head moved off REVIEWED_SHA, and 2 on a
# usage error. Run it under `moat cmd run --timeout`, which owns the deadline.
set -u

repo=${1:?OWNER/REPO}
number=${2:?PR number}
sha=${3:?reviewed head sha}
interval=${4:-30}

# `gh pr checks` reports on the PR's current head, so the head is compared on
# every poll: without it, a push during the wait gets the old head approved.
while :; do
  head=$(moat gh pr view "$number" -R "$repo" --json headRefOid --jq .headRefOid 2>/dev/null)
  if [ -n "$head" ] && [ "$head" != "$sha" ]; then
    echo "head moved: $sha -> $head"
    exit 3
  fi
  # No checks yet reads as pending: a fresh push registers its runs late.
  state=$(moat gh pr checks "$number" -R "$repo" --json name,bucket --jq '
    if length == 0 then "pending"
    elif any(.[]; .bucket == "fail" or .bucket == "cancel") then
      "fail: " + ([.[] | select(.bucket == "fail" or .bucket == "cancel") | .name] | join(", "))
    elif any(.[]; .bucket == "pending") then "pending"
    else "pass" end' 2>/dev/null)
  case $state in
    pass) echo "all checks passed on $sha"; exit 0 ;;
    fail*) echo "$state"; exit 1 ;;
  esac
  sleep "$interval"
done
