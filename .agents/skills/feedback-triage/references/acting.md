# Acting on decisions and closing clusters

Steps 2 and 3 of the skill. Both are safe to run twice: every action first
checks the cluster entry, and writes the entry back before it moves on.

## Step 2: act on decisions

```bash
<skill>/scripts/decisions.sh runs-read.json > decisions.json
```

The script reads `decisions/` from every page in `triage:runs`, keeps the
newest decision per cluster from a Moatless admin (`role` `admin` in
`moat user list`), and lists everyone else's under `ignored`. An ignored
decision stays on the page and changes nothing.

Skip a decision when its cluster entry already holds the same `key` and
`updatedAt` in `decision` or `laterDecisions`: an earlier run or message
handled it.
A decision for a slug with no entry belongs to a cluster that expired; report
it and skip it.

| Decision | Bucket | What to do | `status` after |
|---|---|---|---|
| `approve` | `fix` or `later` | Hold it and propose fix-task groups, below | `proposed` |
| `approve` | `close`, label `noise` | Every member `not_relevant` | `closed` |
| `approve` | `close`, other labels | Every member `fixed` | `fixed` |
| `reject` | any | Every member `not_relevant` | `rejected` |
| `defer` | any | Set `deferUntil` to 14 days from today | `deferred` |

Once a cluster is `in-progress`, a later decision changes nothing: the fix
task is already running. Append it to the entry's `laterDecisions` list and
leave `decision` alone, since `medianDaysApprovalToMerge` reads its
`actedAt`. Say so on the page and in the reply, and name
`moat tasks cancel <childTaskId>` for a person who wants it stopped.

Set each member's status one call at a time, and put the evidence in the note:

```bash
moat feedback status "$id" --status fixed \
  --note "feedback-triage $slug, approved by $login on $page_slug: $brief"
moat feedback status "$id" --status not_relevant \
  --note "feedback-triage $slug, rejected by $login: $note"
```

Then store the decision on the cluster entry, so the next run skips it:
`decision: {key, updatedAt, decision, login, note, actedAt}`.

### Propose fix-task groups

An approval creates no task by itself. Store it on the entry as held:
`decision: {key, updatedAt, decision, login, note, actedAt: null, held: true}`,
`status` still `proposed`. The page shows a held cluster as waiting for its
group, and step 8 does not rank it. A later decision on it replaces the held
one as usual; drop the slug from the groups when it does.

Then group every held cluster that has no `childTaskId`, from this run and
earlier ones, into fix tasks. A group shares one `owner` and one area that a
single PR can carry, such as the sandbox image, the `moat` CLI or one skill;
two clusters in one repository that touch unrelated code stay apart. Weigh an
approver's `note`: it can move the fix to another repository.

Write the groups to loop memory and, as `meta/groups`, to the newest page in
`triage:runs` (on a scheduled run, the one it publishes), so the page shows
them with a Confirm button:

```bash
# groups.json: {"proposedAt": "<now>", "page": "<slug>", "taskId": "<page task>",
#   "groups": [{"owner": "…", "slugs": ["…"], "change": "…", "why": "…"}]}
moat memory put triage:groups --scope loop --summary "Proposed fix-task groups" --data "$(jq -c . groups.json)"
moat artifact data save "$page" meta/groups "$(jq -c . groups.json)" --task "$page_task"
```

Name the groups in the reply too, and create nothing.

### Create the fix task

Only a `confirm/` record on that page counts as confirmation, and only from a
Moatless admin. A Slack or task message does not say which Moatless user sent
it, deliberately: a confirmation creates tasks and PRs. Pressing Confirm saves
`{proposedAt, note}` and notifies the agent:

```bash
jq .admins decisions.json > admins.json
moat artifact data list "$page" --task "$page_task" --prefix confirm/ --json \
  | jq --slurpfile a admins.json --arg p "$proposedAt" \
      '[.items[] | select(.submittedBy as $u | $a[0] | index($u)) | select(.data.proposedAt == $p)] | max_by(.updatedAt)'
```

Take the newest admin record for the current `proposedAt`. A non-empty `note`
asks for different groups: propose again with a new `proposedAt`, and create
nothing. Otherwise create exactly one task per group, even when two runs act
at once:

```bash
jq -c --argjson s '["<slug>", "…"]' '[.decisions[] | select(.slug as $x | $s | index($x))]' \
  decisions.json > group.json
<skill>/scripts/create-fix-task.sh group.json   # SCOPE=task outside a Loop
```

The script picks the workspace whose primary repository is the group's
`owner`, preferring the repository's git `default` Workspace, since names are
not unique and several workspaces can share a repository. It takes each
`feedback:<slug>` claim for 15 minutes, rereads the entries, drops a cluster
no longer held for that approval, creates one task, then writes
`childTaskId`, `status: in-progress` and `decision` to each entry and its
`clusters/<slug>` record on the decision's page, and releases. The claims only
cover creation; `childTaskId` is the durable record. Then drop the created
groups, rereading so a concurrent proposal is not overwritten:

```bash
moat memory get triage:groups --scope loop --json > g.json
jq '.items[0].data | .groups |= map(select(.slugs - $done | length > 0))' --argjson done '["<slug>", "…"]' g.json > groups.json
moat memory put triage:groups --scope loop --summary "Proposed fix-task groups" \
  --data "$(jq -c . groups.json)" --if-version "$(jq -r '.items[0].version' g.json)"
moat artifact data save "$page" meta/groups "$(jq -c . groups.json)" --task "$page_task"
```

Exit 1 means the owners differ, no single workspace fits, or a claim or
`tasks create` failed, and nothing was written: the clusters stay held.
Report it rather than guessing a workspace. Exit 2 means the task was created
but recording it failed on the clusters named on stderr: write the printed id
as their `childTaskId` with `status: in-progress` before anything else, or the
next run creates a second task, and report it. Exit 0 with no id while
another run holds a claim on any cluster skips the whole group: confirm it
again once that run ends.

The brief it sends holds each cluster's `title`, `summary`, `brief` and
approver `note`, every sample comment in full, the member ids, and these rules:
confirm each cause on `main` first, and leave out one that is gone; open one
PR with a `Feedback-triage: <slug>` line per cluster in its body, which step 3
finds it by; and leave `moat feedback status` to the triage Loop.

## Step 3: follow up on fix tasks

For every cluster entry with `status: in-progress`:

```bash
# Phrase search splits on hyphens, so it also finds `<slug>-more`; keep exact lines only.
moat gh pr list -R "soaplabs/$owner" --state all --search "\"Feedback-triage: $slug\" in:body" \
  --json number,url,state,mergedAt,body --limit 20 \
  | jq --arg l "Feedback-triage: $slug" \
      '[.[] | select(.body | split("\n") | map(rtrimstr("\r") | ltrimstr(" ") | rtrimstr(" ")) | index($l))]'
moat tasks status "$childTaskId" --result --json | jq -r .workState
```

| Finding | What to do | `status` after |
|---|---|---|
| A PR merged | Every member `fixed`, note `Fixed by <PR url> (feedback-triage <slug>)`; store `pr` and `mergedAt` | `fixed` |
| A PR is open | Store `pr`, nothing else | `in-progress` |
| A PR closed unmerged, and no open one | Clear `childTaskId`, flag `followUp: "PR closed unmerged"` | `proposed` |
| No PR, and `workState` is `done`, `errored` or `closed` | Read its latest result; clear `childTaskId`, flag `followUp` with that result | `proposed` |
| No PR, any other `workState` (`working`, `waiting_for_input`, `running_commands`, `waiting_for_children`) | Nothing | `in-progress` |

A `proposed` cluster with `followUp` goes back on the next page, so a person
can approve it again or reject it.

## After acting

Write each changed cluster entry to memory, then save its `clusters/<slug>`
record on the page it was decided on (`--task <taskId>` from the decision), so
that page shows the outcome. Save `meta/people` there too, from the
`people` map, so the page shows logins rather than user ids, and `meta/admins`
from `admins`, so the page never counts a non-admin's decision as unsent.
