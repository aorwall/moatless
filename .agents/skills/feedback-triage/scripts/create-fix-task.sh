#!/usr/bin/env bash
# Create the one fix task for a confirmed group of approved clusters, and record
# it on each:
#
#   create-fix-task.sh <decisions.json>
#
# <decisions.json> is one element of `decisions.sh`'s `decisions`, or an array
# of them whose clusters share an `owner`. Prints the new task id. Exit 0
# without a task id: no cluster is still held for its approval, or another run
# holds a claim on any one of them, which skips the whole group. Exit 1: the
# owners differ, no workspace fits, or a claim or `tasks create` failed;
# nothing was created or written. Exit 2: the task was created and its id
# printed, but recording it failed on a cluster named on stderr; set its
# `childTaskId` by hand.
set -euo pipefail

scope="${SCOPE:-loop}"
dir=$(mktemp -d)
claimed=()
cleanup() {
  for s in "${claimed[@]}"; do moat memory release "feedback:$s" --scope "$scope" >/dev/null || true; done
  rm -rf "$dir"
}
trap cleanup EXIT

jq -c 'if type == "array" then . else [.] end' "$1" >"$dir/decisions.json"
mapfile -t slugs < <(jq -r '.[].slug' "$dir/decisions.json")
((${#slugs[@]} > 0)) || { echo "create-fix-task.sh: no decisions in $1" >&2; exit 1; }

entry() { moat memory get "triage:cluster:$1" --scope "$scope" --json | jq '.items[0]'; }
for s in "${slugs[@]}"; do entry "$s" >"$dir/entry-$s.json"; done
owner=$(jq -rs 'map(.data.owner) | unique | if length == 1 then .[0] else "" end' "$dir"/entry-*.json)
if [[ -z "$owner" ]]; then
  echo "create-fix-task.sh: ${slugs[*]} have different owners; one task fixes one repository" >&2
  exit 1
fi

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
  echo "create-fix-task.sh: no single workspace for $owner; ${slugs[*]} stay proposed" >&2
  exit 1
fi

# `claim` 404s on a key nobody has written, hence the `put` first.
for s in "${slugs[@]}"; do
  moat memory put "feedback:$s" --scope "$scope" --summary "Fix task for $s" --data '{}' >/dev/null
  if ! moat memory claim "feedback:$s" --scope "$scope" --ttl 15m >/dev/null 2>"$dir/claim.err"; then
    # 409 is a claim held by another run; anything else is a real failure.
    if grep -q '(409)' "$dir/claim.err"; then
      echo "create-fix-task.sh: another run is creating the task for $s" >&2
      exit 0
    fi
    cat "$dir/claim.err" >&2
    exit 1
  fi
  claimed+=("$s")
done

# Reread under the claims: a cluster another run gave a task, or whose held
# approval a later reject, defer or approve replaced, leaves the group.
todo=()
for s in "${slugs[@]}"; do
  entry "$s" >"$dir/entry-$s.json"
  dec=$(jq -c --arg s "$s" '.[] | select(.slug == $s)' "$dir/decisions.json")
  if ! jq -e --argjson d "$dec" '.data.childTaskId == null and .data.decision.held == true
      and .data.decision.key == $d.key and .data.decision.updatedAt == $d.updatedAt
      and $d.decision == "approve"' "$dir/entry-$s.json" >/dev/null; then
    echo "create-fix-task.sh: $s is not held for this approval; left out of this task" >&2
  else
    todo+=("$s")
  fi
done
((${#todo[@]} > 0)) || exit 0

for s in "${todo[@]}"; do cat "$dir/entry-$s.json"; done | jq -rs '
  map(.data) as $c
  | (if ($c | length) == 1 then "# \($c[0].title)\n"
     else "# Fix \($c | length) feedback clusters in \($c[0].owner)\n" end),
    ($c[] | "## \(.title)\n\n\(.summary)\n\n**Proposed fix:** \(.brief)\n",
      (if .decision.note // "" | length > 0 then "**Approver note:** \(.decision.note)\n" else empty end),
      "### Reports\n",
      (.samples[]? | "> \(.comment | gsub("\n"; "\n> "))\n>\n> feedback \(.feedbackId)\n"),
      "Member feedback ids: \(.members | join(", "))\n"),
    "## How to work this\n",
    "The feedback-triage Loop clustered these reports and a Moatless admin approved a fix.\n",
    "- Confirm each cause on `main` first. If one is already gone, leave it out of the PR and say so in your final reply, naming the commit that fixed it.",
    "- Open one PR with the fixes, and put these lines in its body, one per cluster it fixes. The triage Loop finds the PR by them:",
    ($c[] | "  `Feedback-triage: \(.slug)`"),
    "- Do not run `moat feedback status`: the triage Loop closes these items when the PR merges."' \
  >"$dir/brief.md"

if ((${#todo[@]} == 1)); then
  name="Fix: $(jq -r .data.title "$dir/entry-${todo[0]}.json")"
else
  name="Fix: ${#todo[@]} feedback clusters in $owner"
fi
tid=$(moat tasks create --workspace "$ws" --agent-profile claude-code \
  --name "$name" --message "$(cat "$dir/brief.md")" --json | jq -r '.taskId // empty')
if [[ ! "$tid" =~ ^[0-9a-f-]{36}$ ]]; then
  echo "create-fix-task.sh: tasks create failed for ${todo[*]}; they stay proposed" >&2
  exit 1
fi

# The task exists now. Another run may rewrite an entry meanwhile, so retry a
# refused `--if-version` on a fresh read; never exit without printing $tid.
now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
failed=""
for s in "${todo[@]}"; do
  dec=$(jq -c --arg s "$s" '.[] | select(.slug == $s)' "$dir/decisions.json")
  recorded=""
  for _ in 1 2 3; do
    entry "$s" >"$dir/entry.json" || continue
    jq --arg t "$tid" --arg now "$now" --argjson d "$dec" \
      '.data + {status: "in-progress", childTaskId: $t, followUp: null,
                decision: {key: $d.key, updatedAt: $d.updatedAt, decision: $d.decision,
                           login: $d.login, note: $d.note, actedAt: $now}}' \
      "$dir/entry.json" >"$dir/data.json"
    if moat memory put "triage:cluster:$s" --scope "$scope" --summary "$(jq -r .summary "$dir/entry.json")" \
      --data "$(jq -c . "$dir/data.json")" --if-version "$(jq -r .version "$dir/entry.json")" >/dev/null; then
      recorded=1
      break
    fi
  done
  if [[ -z "$recorded" ]]; then
    echo "create-fix-task.sh: created $tid but could not record it on $s; set childTaskId by hand" >&2
    failed=1
    continue
  fi

  page=$(jq -r .pageSlug <<<"$dec")
  task=$(jq -r .taskId <<<"$dec")
  # The page record only displays the outcome; the entry above is what counts.
  if ! { moat artifact data list "$page" --task "$task" --prefix "clusters/$s" --json \
    | jq --arg k "clusters/$s" --argjson x "$(jq -c '{status, childTaskId, decision, followUp}' "$dir/data.json")" \
        '.items[] | select(.key == $k) | .data + $x' >"$dir/record.json" \
    && [[ -s "$dir/record.json" ]] \
    && moat artifact data save "$page" "clusters/$s" "$(jq -c . "$dir/record.json")" --task "$task" >/dev/null; }; then
    echo "create-fix-task.sh: recorded $tid, but could not update clusters/$s on $page" >&2
  fi
done
echo "$tid"
[[ -z "$failed" ]] || exit 2
