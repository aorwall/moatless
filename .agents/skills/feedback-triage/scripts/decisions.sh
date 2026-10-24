#!/usr/bin/env bash
# Print the decisions saved on every triage page as one JSON object:
#
#   decisions.sh <runs.json>
#
#   {"decisions": [the newest admin decision per cluster],
#    "ignored":   [decisions from users who are not Moatless admins],
#    "people":    {"<user id>": "<login>"} for everyone who decided,
#    "admins":    [the user id of every Moatless admin]}
#
# <runs.json> is the `triage:runs` data. A page that cannot be read is reported
# on stderr and skipped, so one deleted page does not hide the others.
set -euo pipefail

runs="$1"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT

moat user list --json >"$dir/users.json"
i=0
while read -r slug task; do
  [[ -n "$slug" ]] || continue
  if moat artifact data list "$slug" --task "$task" --prefix decisions/ --json >"$dir/raw.json"; then
    jq --arg page "$slug" --arg task "$task" \
      '[.items[] | . + {pageSlug: $page, taskId: $task}]' "$dir/raw.json" >"$dir/page-$i.json"
    i=$((i + 1))
  else
    echo "decisions.sh: cannot read $slug on task $task" >&2
  fi
done < <(jq -r '.runs[]? | "\(.pageSlug) \(.taskId)"' "$runs" | sort -u)

if ((i == 0)); then
  echo '{"decisions": [], "ignored": [], "people": {}, "admins": []}'
  exit 0
fi

# The cluster slug is the key's middle segment: decisions/<slug>/<record id>.
jq -s --slurpfile users "$dir/users.json" '
  ($users[0].users | map({key: .id, value: .}) | from_entries) as $u
  | (map(.[]) | map(. + {slug: (.key | split("/")[1]),
                         login: ($u[.submittedBy].login // .submittedBy),
                         admin: ($u[.submittedBy].role == "admin")})) as $all
  | {decisions: ($all | map(select(.admin)) | group_by(.slug)
                 | map(max_by(.updatedAt)
                       | {slug, key, decision: .data.decision, note: (.data.note // ""),
                          by: .submittedBy, login, updatedAt, pageSlug, taskId})),
     ignored: ($all | map(select(.admin | not)) | map({slug, key, login, decision: .data.decision})),
     people: ($all | map({key: .submittedBy, value: .login}) | from_entries),
     admins: ($users[0].users | map(select(.role == "admin") | .id))}
' "$dir"/page-*.json
