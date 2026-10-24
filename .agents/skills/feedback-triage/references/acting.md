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
| `approve` | `fix` or `later` | Create one fix task, below | `in-progress` |
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

### Create the fix task

Exactly one task per approved cluster, even when two runs act at once:

```bash
jq -c --arg s "$slug" '.decisions[] | select(.slug == $s)' decisions.json > decision.json
<skill>/scripts/create-fix-task.sh decision.json   # SCOPE=task outside a Loop
```

The script picks the workspace whose primary repository is the cluster's
`owner`, preferring the one named exactly after it, since names are not unique
and several workspaces can share a repository. It takes the `feedback:<slug>`
claim for 15 minutes, rereads the entry, creates the task, then writes
`childTaskId`, `status: in-progress` and `decision` to the entry and to the
`clusters/<slug>` record on the decision's page, and releases. The claim only
covers creation; `childTaskId` is the durable record.

Exit 1 means no single workspace fits, or the claim or `tasks create` failed,
and nothing was written: the cluster stays `proposed`. Report it on the page
and in the reply rather than guessing a workspace. Exit 2 means the task was
created but recording it failed: write the printed id as `childTaskId` with
`status: in-progress` before anything else, or the next run creates a second
task, and report it.

The brief it sends holds the cluster's `title`, `summary` and `brief`, every
sample comment in full, the member ids, and these rules: confirm the cause on
`main` first, and open no PR if it is gone; open one PR with the line
`Feedback-triage: <slug>` in its body, which step 3 finds it by; and leave
`moat feedback status` to the triage Loop.

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
