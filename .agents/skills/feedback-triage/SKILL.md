---
name: feedback-triage
description: "Cluster open Moatless feedback by root cause, name the repository that owns each fix, publish the run's approval Artifact and post it to Slack, then act on a Moatless admin's decisions: propose fix-task groups, create one task per confirmed group, and close the items when its PR merges. Use for a feedback-triage Loop run, for a message from a triage page (`<source>artifact</source>`), or when asked to group, triage or cluster the feedback inbox (`moat feedback list`)."
---

# Feedback triage

Each run takes the open feedback from the last seven days, groups it into
clusters that share one root cause, proposes one fix per cluster in the
repository that owns it, and publishes the clusters on a new approval page. A
person reads the page, saves approve, reject or defer on a cluster, and
presses Notify the agent to have the decisions acted on before the next run.
A Moatless admin's approvals are grouped into proposed fix tasks, one
repository and one area each; once an admin confirms the groups, each becomes
one task in the owning repository's workspace, and the items close when that
task's PR merges.

Older open feedback is left alone. Items stay open after their week passes,
and no run reads them again: what matters is what agents hit now, and a cause
that still bites shows up again in fresh feedback.

```
1. State      → known clusters and earlier runs from loop memory
2. Decide     → act on admin decisions saved on earlier pages
3. Follow up  → close clusters whose fix PR merged
4. Batch      → open items from the last 7 days
5. Label      → one first label per item
6. Cluster    → join known clusters, then group the rest by root cause
7. Check      → open or merged work in the owning repository
8. Rank       → at most 10 clusters to fix first
9. Publish    → the page, then its records
10. Record    → cluster entries, then the run
11. Slack     → a short summary and the page link to the triage channel
```

**A message from a page** (`<source>artifact</source>`) means someone pressed
Notify the agent: a `notify/` record lists the decisions they sent. Run steps
1 to 3 only, then stop: the next scheduled run publishes the next page. Step 2
reads every decision anyway, so a decision saved but not sent is acted on too.

When a page message names a `confirm/` record, someone pressed Confirm on the
proposed fix-task groups: follow `references/acting.md`, *Create the fix
task*, which checks that an admin saved it.

## 1. Read the state

All state is loop-scoped memory. Pass `--scope loop` on every read and write,
or a read also returns same-named entries from the Workspace.

| Key | `--data` holds |
|---|---|
| `triage:runs` | `{runs: [{date, taskId, pageSlug, processed}]}`, newest last |
| `triage:cluster:<slug>` | One cluster, shaped as below |
| `triage:groups` | Fix-task groups awaiting confirmation; `references/acting.md` |

```bash
moat memory get triage:runs --scope loop --json > runs-get.json \
  && jq '.items[0].data' runs-get.json > runs-read.json || echo '{}' > runs-read.json
moat memory get triage:groups --scope loop --json > groups-get.json || echo '{}' > groups-get.json
moat memory list --prefix triage:cluster: --scope loop --limit 500 --json | jq -r '.items[].key' > known-keys.txt
mkdir -p known && while read -r k; do
  moat memory get "$k" --scope loop --json | jq '.items[0].data' > "known/${k#triage:cluster:}.json"
done < known-keys.txt
```

A `get` for a key that does not exist fails with `404 Memory entry not found`:
on the first run that means no clusters yet, not an error. Read the clusters
one key at a time: `get 'triage:cluster:*'` stops at 50 entries
(`MAX_PREFIX_GET`) with no error, and weeks of runs make more clusters than
that. `list` stops at 500; a listing of exactly 500 keys is truncated, so stop
and report it rather than cluster against a partial set.

Each cluster is its own entry rather than one entry holding them all: a memory
body or `--data` is at most 16 KiB (`MAX_BODY_BYTES`), which all clusters'
samples together overflow.

A cluster entry's data, which is also the page record `clusters/<slug>`:

```json
{"slug": "sandbox-restart-loses-build-state", "title": "…", "label": "tool-bug",
 "owner": "moatless", "ownerReason": "…", "reportedFrom": ["moatless"],
 "summary": "…", "brief": "…", "existing": [{"label": "#953 merged", "url": "…"}],
 "members": ["<feedbackId>", "…"], "samples": [{"feedbackId": "…", "comment": "…"}],
 "count": 3, "firstSeen": "<createdAt>", "lastSeen": "<createdAt>",
 "bucket": "fix", "priority": {"impact": 4, "ease": 2, "score": 6, "rank": 1,
 "why": "…", "reasons": {"impact": "…", "ease": "…"}},
 "status": "proposed", "pages": ["feedback-triage-2026-10-26"],
 "decision": {"key": "…", "updatedAt": "…", "decision": "approve", "login": "…",
 "note": "", "actedAt": "…"}, "laterDecisions": [], "childTaskId": "…",
 "pr": "…", "mergedAt": "…",
 "deferUntil": "<YYYY-MM-DD>", "followUp": "…", "recurred": false}
```

`status` is `proposed`, `in-progress`, `fixed`, `closed`, `rejected` or
`deferred`. Steps 2 and 3 set it; step 6 sets it back to `proposed` when a
closed cause comes back.

The feedback API shows a caller who is not a Moatless admin only the feedback
on tasks it can access. Check that first:

```bash
moat whoami --json | jq -r .user.role
```

Anything but `admin` means the batch is a partial inbox. Stop and report that
the Loop's run-as user must be an admin, rather than publishing clusters that
miss most of the feedback.

## 2. Act on decisions

Follow `references/acting.md`, step 2. `scripts/decisions.sh` reads every
earlier page's decisions and keeps the newest from a Moatless admin per
cluster; a decision from anyone else changes nothing. Approvals that need a
fix are held and proposed as fix-task groups; no task is created until an
admin confirms them.

## 3. Follow up on fix tasks

Follow `references/acting.md`, step 3: find each `in-progress` cluster's PR by
the `Feedback-triage: <slug>` line in its body, and close the members as
`fixed` once it merges.

## 4. Take the batch

```bash
<skill>/scripts/batch.sh 7 > batch.json
moat feedback list --status open --json --limit 1 | jq .total   # openTotal
```

The batch is every open item created in the last seven days, oldest first.
Runs overlap, so one item lands on up to five weekday pages; each run clusters
its window from scratch rather than carrying anything forward. An empty batch
is a normal result: publish a page that says so and record the run.

## 5. Give each item a first label

Read every comment in the batch and give it one label:

| Label | The comment reports |
|---|---|
| `tool-bug` | A defect in a tool, CLI, script, build, test setup or the sandbox that code or config must fix |
| `instruction-gap` | A skill, `AGENTS.md`, `CLAUDE.md` or doc that is missing, wrong or stale |
| `feature` | Something nobody built yet, asked for as new behaviour |
| `noise` | Nothing to fix: a one-off, a retraction, or the reporter's own mistake |

The label only seeds step 6, so a wrong one costs little.

## 6. Cluster

1. **Match each item against the known clusters first.**

   An item with the same root cause as a known cluster reuses its slug,
   whatever its label: a slug is stable for the life of the cluster, and a
   later run finds earlier decisions under it. `members`, `count`, `samples`,
   `firstSeen` and `lastSeen` describe this window only, so rewrite them from
   the batch rather than adding to the stored ones. A known cluster with no
   member in the window stays in memory and off the page.

   The known cluster's `status` decides what a match does:

   | `status` | A new member |
   |---|---|
   | `proposed` | Joins it as usual |
   | `in-progress` | Joins it; the fix is coming, and step 3 closes it with the rest |
   | `deferred`, `deferUntil` not yet reached | Joins it, and the cluster stays off the page |
   | `deferred`, `deferUntil` reached | Joins it and reopens it: `status: proposed`, `deferUntil` cleared |
   | `fixed`, `closed` or `rejected` | Reopens it: `status: proposed`, `recurred: true`, keep `decision`, `pr` and `mergedAt` so the page shows the earlier outcome |

   A reopen also sets `bucket`, `priority.rank` and `childTaskId` to null:
   step 8 skips a cluster left in `close`, and `scripts/create-fix-task.sh`
   leaves out any cluster whose `childTaskId` is set, so a stale one is never
   fixed again.
2. **Group the rest by owner and label, then merge groups that share a root
   cause.**

   Name the owner from the comment, not from `reportedFrom`; see
   `references/owners.md`. A root cause is the one change that would stop every
   member recurring. Two complaints about one skill that need two unrelated
   edits are two clusters.
3. **Write each new cluster.**

   The slug is lower-case kebab naming the cause, such as
   `sandbox-restart-loses-build-state`, not the symptom or the reporter.
   `summary` says what members hit and what it cost them. `brief` is the change
   a fix task would make, naming the file or command when a comment names it.
   Take up to three `samples` that differ from each other, in full.

A singleton is a fine cluster. Do not stretch a cluster to absorb an item that
only resembles it: a person can merge two clusters on the page, but cannot
easily see that one cluster holds two causes.

Put `noise` items in one cluster per owner, `noise-<owner>`, so the page shows
them and a person can reject them in one decision.

## 7. Check for work already done

Before a cluster goes on the page, search its owning repository for a PR on the
same cause:

```bash
moat gh pr list -R soaplabs/<owner> --state all --search "<two or three cause words>" --limit 5 \
  --json number,title,state,mergedAt,url
```

Add each match to `existing` with its state in the label. A PR merged after the
cluster's `lastSeen` likely fixed it: say so in `brief` (*likely fixed by #953;
close the members as fixed*) rather than proposing the same fix again. Then
check the code on `main` to confirm, since a title can claim more than the PR
did.

## 8. Rank what to fix first

Every run ranks every cluster on its page afresh, because counts change as
items arrive and fixes merge. Put each cluster in one `bucket`:

| Bucket | Holds |
|---|---|
| `close` | A merged PR already fixed it, or it is `noise`. The `pr` a `recurred` cluster keeps did not fix it. The decision it needs is close or reject, not a fix |
| `fix` | The top ten by score, at most. Fewer is fine when the rest score low |
| `later` | Everything else, still open for a decision |

Score each `fix` and `later` cluster on two axes:

- `impact`, 1 to 5: how bad one occurrence is, plus how often it happens.
  Start from the harm: 3 when it destroys work, ships a broken or unverified
  change, or blocks a Loop; 2 when it costs a recurring class of tasks real
  time; 1 for one-off friction. Add 1 for 2 to 4 items, or 2 for 5 or more
  items or 3 or more reporting repositories. A single report that shows the
  harm will repeat, such as a Loop that fails on every run or an incident in
  prod, adds 1.
- `ease`, 1 to 3: 3 for a doc, skill or one-line config edit; 2 for a
  contained change in one repository; 1 for infrastructure, several
  repositories, or a design question.

`score` is `impact + ease`, from 2 to 8. Break ties by `impact`, then by
`count`, then by the newest `lastSeen`.

Rank the ten highest-scoring clusters 1 to 10 and give them `bucket: "fix"`.
Set `rank` to null on every other cluster, so one that drops out of the top
ten carries no old rank. Before a cluster takes a rank, confirm its cause is
still on `main`: read the file or run the command its brief names. A cluster
whose cause is gone moves to `close`. Write `why` as one sentence a person can
check, naming the cost and the fix, such as *Six Codex bump runs unverified;
rotate one credential*. It is for the final reply only: the page already shows
the cost in `summary` and `reasons`, and the fix in `brief`.

Give every scored cluster `reasons`: one sentence per axis naming the evidence
behind its score, the harm and the count for `impact`, and the file or system
the fix touches for `ease`. The page shows them under the meter. Write
`brief` for a `fix` cluster as the change itself, naming the file or command:
the page shows it on the card as the suggested change.

Rank only `proposed` clusters that are not held. An `in-progress` cluster
keeps its bucket and shows its task on the page instead of a rank; a held one
keeps its bucket and shows that it waits for its group.

The ranking is a proposal. A person can approve any cluster, ranked or not.

## 9. Publish the page

One page per run, under the slug `feedback-triage-<YYYY-MM-DD>` in UTC. It
lists the clusters with a member in this run's window, except `deferred` ones
whose `deferUntil` is still ahead, plus every `proposed` cluster with a
`followUp` or a held approval. Save `meta/groups` from `triage:groups` too,
so the page shows the groups waiting for confirmation.

```bash
cp <skill>/scripts/page.html page.html
moat artifact publish page.html --title "Feedback triage $(date -u +%F)" \
  --slug "feedback-triage-$(date -u +%F)" --data --rules <skill>/scripts/rules.json --private
```

Then write the records, `meta/run`, `meta/people` and `meta/admins` first:

```bash
moat artifact data save "$slug" meta/run "$(jq -c . meta.json)"
moat artifact data save "$slug" meta/people "$(jq -c .people decisions.json)"
moat artifact data save "$slug" meta/admins "$(jq -c .admins decisions.json)"
moat artifact data save "$slug" meta/groups "$(jq -c '.items[0].data // {}' groups-get.json)"
for f in clusters/*.json; do moat artifact data save "$slug" "clusters/$(basename "$f" .json)" "$(jq -c . "$f")"; done
```

`meta/run` holds `{runDate, windowDays, processed, windowFrom, windowTo,
clusters, openTotal, oldestOpen, closedThisWeek, medianDaysToClose,
medianDaysApprovalToMerge, recurred}`:

- `closedThisWeek` and `medianDaysToClose` come from
  `moat feedback list --status fixed` and `--status not_relevant` with
  `--since` seven days back, using `statusUpdatedAt − createdAt`.
- `medianDaysApprovalToMerge` comes from the cluster entries that reached
  `fixed` through a PR, using `decision.actedAt` and the PR's `mergedAt`.
- `recurred` counts the clusters this run reopened with `recurred: true`: a
  fix that merged but did not hold.

`meta/people` maps user ids to logins, so the page names who decided, and
`meta/admins` lists the admins' ids, so it counts only their decisions as
unsent. The page renders from the records, so a record you correct shows up
without republishing.

- **Publish with `--private`.**

  Feedback carries file paths, task titles and the internals of other
  repositories, and the API filters it by access. A public page would show
  every signed-in user what only its reporters and admins can read.
- **Publish with `--rules`, every time.**

  `scripts/rules.json` lets readers append to `decisions/<slug>/` and change
  only the records they created, and keeps them off `clusters/` and `meta/`.
  A publish without `--rules` drops the rules, and any reader can then
  overwrite a cluster. A `400` naming `unexpected form field "rules"` means
  the backend predates record rules: stop and report that, rather than
  publishing without them. Only `notify/` notifies: a saved decision waits
  until a reader presses Notify the agent (the message entry above) or the
  next scheduled run, so a reader can save several and send them at once.
- **Read who decided from a decision's `submittedBy`.**

  The page appends each reader's first decision under a server-made id and
  updates that record afterwards, so a reader cannot claim the key another
  reader would save under. Two open tabs can still give one reader two
  records for a cluster: `decisions.sh` takes the newest by `updatedAt`.

## 10. Record the run

Write the clusters before the run, so a run listed in `triage:runs` always has
its clusters stored:

```bash
moat memory put "triage:cluster:$c" --scope loop --summary "<count> · <owner> · <title>" --data "$(jq -c . clusters/$c.json)"
jq --arg t "$TASK_ID" --arg slug "$slug" --argjson n "$(jq length batch.json)" \
  '.runs = ((.runs // []) + [{date: (now | strftime("%Y-%m-%d")), taskId: $t, pageSlug: $slug, processed: $n}])[-60:]' \
  runs-read.json > runs.json
moat memory put triage:runs --scope loop --no-expiry --summary "Feedback triage runs and their pages" --data "$(jq -c . runs.json)"
```

Write cluster entries without `--no-expiry`. Each write restarts the 28-day
expiry, so a cluster lasts while its cause keeps being reported and expires
after four quiet weeks; with `--no-expiry` they pile up until `list` hits 500
and step 1 stops every run. A cause that returns after that gets a new slug.
Write an `in-progress` cluster every run even when no item joined it, so it
cannot expire while its fix task works.

`triage:runs` keeps the newest 60 runs, which stays well under 16 KiB. A page
lives on the task that published it, so reading its decisions later takes
`moat artifact data list <pageSlug> --task <taskId>`.

## 11. Post to Slack

After a full run, post a short summary and the page link to the triage
channel, `C0000000004`:

```bash
channel=C0000000004
url=$(moat artifact list --json | jq -r --arg s "$slug" '.items[] | select(.slug == $s) | .latestUrl')
moat send --to "channel:$channel" --via slack --text "$(cat slack.txt)"
```

`latestUrl` opens the newest version, so the link survives a republish. Keep
the text to five lines a person reads in Slack without opening the page:

```
Feedback triage, last 7 days: 120 items in 78 clusters, 10 to fix first.
Top 3: <title> (<owner>), <title> (<owner>), <title> (<owner>).
Since the last run: 2 fix tasks created, 1 PR merged, 5 items closed.
Awaiting confirmation: 3 fix-task groups for 5 approved clusters.
Approve or reject on the page, then press Notify the agent: <url>
```

Leave out a line with nothing to say, such as the third on a run with no
decisions. A run started by a page message posts nothing: the person who
pressed Notify the agent is on the page already. A failed send changes no
state; report it in the reply and go on.

End with one reply: what steps 2 and 3 did (tasks created, items closed,
decisions ignored and why, fix-task groups awaiting confirmation), the page
link, how many items and clusters, the
ranked clusters with their owners and `why`, and how many clusters to close.

## Done when

- The page is published with `--data --rules … --private`, and every
  cluster on it has a record.
- Each cluster names an owner, and an owner that differs from every
  `reportedFrom` carries an `ownerReason`.
- Each cluster has a `bucket`, at most ten have `bucket: "fix"`, and each of
  those has a `rank`, a `score` and a `why`.
- Every cluster on the page, then the run, is in loop memory.
- A full run posted its summary and page link to the triage channel.
- Each admin decision since the last run was acted on once: an approval
  needing a fix is held in a proposed group, a task is created only for a
  group an admin confirmed, at most one per cluster, and every member of a
  closed cluster carries a status note that names the cluster and its
  evidence.
