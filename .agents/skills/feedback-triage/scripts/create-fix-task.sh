#!/usr/bin/env bash
# Create the one fix task for an approved cluster, and record it:
#
#   create-fix-task.sh <decision.json>
#
# <decision.json> is one element of `decisions.sh`'s `decisions`. Prints the new
# task id. Exit 0 without a task id: the cluster already has a task, or another
# run holds the claim. Exit 1: no workspace fits, or the claim or `tasks create`
# failed; nothing was created or written. Exit 2: the task was created and its
# id printed, but recording it failed; set `childTaskId` by hand.
set -euo pipefail

scope="${SCOPE:-loop}"
dec=$(jq -c . "$1")
slug=$(jq -r .slug <<<"$dec")
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT

entry() { moat memory get "triage:cluster:$slug" --scope "$scope" --json | jq '.items[0]'; }
entry >"$dir/entry.json"
owner=$(jq -r .data.owner "$dir/entry.json")

# Names are not unique, and several workspaces can share a primary repository.
# The repository's git `default` Workspace comes first: a name match can land
# on a manual row carrying an admin identity.
rid=$(moat repos list --json | jq -r --arg o "$owner" '.[] | select(.name == $o) | .id')
ws=$(moat workspaces list --json | jq -r --arg id "$rid" --arg o "$owner" '
  [.[] | select(any(.repos[]; .isPrimary and .repositoryId == $id))] as $w
  | ([$w[] | select(.source == "git" and .sourceRepositoryId == $id and .sourceKey == "default")]
      | if length == 1 then .[0].id else empty end)
    // ([$w[] | select(.name == $o)] | if length == 1 then .[0].id else empty end)
    // (if ($w | length) == 1 then $w[0].id else empty end) // ""')
if [[ -z "$ws" ]]; then
  echo "create-fix-task.sh: no single workspace for $owner; $slug stays proposed" >&2
  exit 1
fi

jq -r '.data | "# \(.title)\n\n\(.summary)\n\n**Proposed fix:** \(.brief)\n\n## Reports\n",
  (.samples[]? | "> \(.comment | gsub("\n"; "\n> "))\n>\n> feedback \(.feedbackId)\n"),
  "Member feedback ids: \(.members | join(", "))\n",
  "## How to work this\n",
  "The feedback-triage Loop clustered these reports and a Moatless admin approved a fix.\n",
  "- Confirm the cause on `main` first. If it is already gone, open no PR and say so in your final reply, naming the commit that fixed it.",
  "- Open one PR with the fix, and put this line in its body: `Feedback-triage: \(.slug)`. The triage Loop finds the PR by that line.",
  "- Do not run `moat feedback status`: the triage Loop closes these items when the PR merges."' \
  "$dir/entry.json" >"$dir/brief.md"

# `claim` 404s on a key nobody has written, hence the `put` first.
moat memory put "feedback:$slug" --scope "$scope" --summary "Fix task for $slug" --data '{}' >/dev/null
if ! moat memory claim "feedback:$slug" --scope "$scope" --ttl 15m >/dev/null 2>"$dir/claim.err"; then
  # 409 is a claim held by another run; anything else is a real failure.
  if grep -q '(409)' "$dir/claim.err"; then
    echo "create-fix-task.sh: another run is creating the task for $slug" >&2
    exit 0
  fi
  cat "$dir/claim.err" >&2
  exit 1
fi
trap 'moat memory release "feedback:$slug" --scope "$scope" >/dev/null; rm -rf "$dir"' EXIT

entry >"$dir/entry.json"
if [[ "$(jq -r '.data.childTaskId // empty' "$dir/entry.json")" != "" ]]; then
  echo "create-fix-task.sh: $slug already has a fix task" >&2
  exit 0
fi

tid=$(moat tasks create --workspace "$ws" --agent-profile claude-code \
  --name "Fix: $(jq -r .data.title "$dir/entry.json")" --message "$(cat "$dir/brief.md")" --json | jq -r '.taskId // empty')
if [[ ! "$tid" =~ ^[0-9a-f-]{36}$ ]]; then
  echo "create-fix-task.sh: tasks create failed for $slug; it stays proposed" >&2
  exit 1
fi

# The task exists now. Another run may rewrite the entry meanwhile, so retry a
# refused `--if-version` on a fresh read; never exit without printing $tid.
now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
recorded=""
for _ in 1 2 3; do
  entry >"$dir/entry.json" || continue
  jq --arg t "$tid" --arg now "$now" --argjson d "$dec" \
    '.data + {status: "in-progress", childTaskId: $t,
              decision: {key: $d.key, updatedAt: $d.updatedAt, decision: $d.decision,
                         login: $d.login, note: $d.note, actedAt: $now}}' \
    "$dir/entry.json" >"$dir/data.json"
  if moat memory put "triage:cluster:$slug" --scope "$scope" --summary "$(jq -r .summary "$dir/entry.json")" \
    --data "$(jq -c . "$dir/data.json")" --if-version "$(jq -r .version "$dir/entry.json")" >/dev/null; then
    recorded=1
    break
  fi
done
if [[ -z "$recorded" ]]; then
  echo "create-fix-task.sh: created $tid but could not record it on $slug; set childTaskId by hand" >&2
  echo "$tid"
  exit 2
fi

page=$(jq -r .pageSlug <<<"$dec")
task=$(jq -r .taskId <<<"$dec")
# The page record only displays the outcome; the entry above is what counts.
if ! { moat artifact data list "$page" --task "$task" --prefix "clusters/$slug" --json \
  | jq --arg k "clusters/$slug" --argjson x "$(jq -c '{status, childTaskId, decision}' "$dir/data.json")" \
      '.items[] | select(.key == $k) | .data + $x' >"$dir/record.json" \
  && [[ -s "$dir/record.json" ]] \
  && moat artifact data save "$page" "clusters/$slug" "$(jq -c . "$dir/record.json")" --task "$task" >/dev/null; }; then
  echo "create-fix-task.sh: recorded $tid, but could not update clusters/$slug on $page" >&2
fi
echo "$tid"
