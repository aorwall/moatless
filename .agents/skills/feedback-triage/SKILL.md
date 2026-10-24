---
name: feedback-triage
description: "Cluster open Moatless feedback by root cause, name the repository that owns each fix, and publish the run's approval Artifact. Use for a feedback-triage Loop run, or when asked to group, triage or cluster the feedback inbox (`moat feedback list`). Read-only: it never creates a task and never changes a feedback status."
---

# Feedback triage

Each run takes the open feedback from the last seven days, groups it into
clusters that share one root cause, proposes one fix per cluster in the
repository that owns it, and publishes the clusters on a new approval page. A
person reads the page and records approve, reject or defer on each cluster.

**This skill is read-only.** Do not create a task, do not run `moat feedback
status`, and do not act on a decision saved on any page. The page collects
decisions so a few runs can be reviewed before approval handling is built.

Older open feedback is left alone. Items stay open after their week passes,
and no run reads them again: what matters is what agents hit now, and a cause
that still bites shows up again in fresh feedback.

```
1. State      → known clusters from loop memory
2. Batch      → open items from the last 7 days
3. Label      → one first label per item
4. Cluster    → join known clusters, then group the rest by root cause
5. Check      → open or merged work in the owning repository
6. Rank       → at most 10 clusters to fix first
7. Publish    → the page, then its records
8. Record     → cluster entries, then the run
```

## 1. Read the state

All state is loop-scoped memory. Pass `--scope loop` on every read and write,
or a read also returns same-named entries from the Workspace.

| Key | `--data` holds |
|---|---|
| `triage:runs` | `{runs: [{date, taskId, pageSlug, processed}]}`, newest last |
| `triage:cluster:<slug>` | One cluster, shaped as below |

```bash
moat memory get triage:runs --scope loop --json > runs-get.json \
  && jq '.items[0].data' runs-get.json > runs-read.json || echo '{}' > runs-read.json
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
 "bucket": "fix", "priority": {"impact": 3, "reach": 3, "urgency": 2, "ease": 2,
 "score": 13, "rank": 1, "why": "…"},
 "status": "proposed", "pages": ["feedback-triage-2026-10-26"]}
```

The feedback API shows a caller who is not a Moatless admin only the feedback
on tasks it can access. Check that first:

```bash
moat whoami --json | jq -r .user.role
```

Anything but `admin` means the batch is a partial inbox. Stop and report that
the Loop's run-as user must be an admin, rather than publishing clusters that
miss most of the feedback.

## 2. Take the batch

```bash
<skill>/scripts/batch.sh 7 > batch.json
moat feedback list --status open --json --limit 1 | jq .total   # openTotal
```

The batch is every open item created in the last seven days, oldest first.
Runs overlap, so one item lands on up to five weekday pages; each run clusters
its window from scratch rather than carrying anything forward. An empty batch
is a normal result: publish a page that says so and record the run.

## 3. Give each item a first label

Read every comment in the batch and give it one label:

| Label | The comment reports |
|---|---|
| `tool-bug` | A defect in a tool, CLI, script, build, test setup or the sandbox that code or config must fix |
| `instruction-gap` | A skill, `AGENTS.md`, `CLAUDE.md` or doc that is missing, wrong or stale |
| `feature` | Something nobody built yet, asked for as new behaviour |
| `noise` | Nothing to fix: a one-off, a retraction, or the reporter's own mistake |

The label only seeds step 4, so a wrong one costs little.

## 4. Cluster

1. **Match each item against the known clusters first.**

   An item with the same root cause as a known cluster reuses its slug,
   whatever its label: a slug is stable for the life of the cluster, and a
   later run finds earlier decisions under it. `members`, `count`, `samples`,
   `firstSeen` and `lastSeen` describe this window only, so rewrite them from
   the batch rather than adding to the stored ones. A known cluster with no
   member in the window stays in memory and off the page.
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

## 5. Check for work already done

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

## 6. Rank what to fix first

Every run ranks every cluster on its page afresh, because reach and urgency
change as items arrive and fixes merge. Put each cluster in one `bucket`:

| Bucket | Holds |
|---|---|
| `close` | A merged PR already fixed it, or it is `noise`. The decision it needs is close or reject, not a fix |
| `fix` | The top ten by score, at most. Fewer is fine when the rest score low |
| `later` | Everything else, still open for a decision |

Score each `fix` and `later` cluster from 1 to 3 on four axes:

| Axis | 3 | 2 | 1 |
|---|---|---|---|
| `impact` | Destroys work, ships unverified changes, or blocks a Loop on every run | Costs a recurring class of tasks real time | One-off friction |
| `reach` | 5 or more items this window, or 3 or more reporting repositories | 2 to 4 items | One item |
| `urgency` | The cause is on `main` today and a Loop, a deadline or a security fix waits on it | The cause is on `main` today | May already be gone |
| `ease` | A doc, skill or one-line config edit | A contained change in one repository | Infrastructure, several repositories, or a design question |

`score` is `2 × impact + reach + urgency + ease`, from 5 to 15. Impact counts
twice because a cheap fix to a harmless problem should not outrank an
expensive fix to one that destroys work. Break ties by `impact`, then by
`count`.

Rank the ten highest-scoring clusters 1 to 10 and give them `bucket: "fix"`.
Set `rank` to null on every other cluster, so one that drops out of the top
ten carries no old rank. Before a cluster takes a rank, confirm its cause is
still on `main`: read the file or run the command its brief names. A cluster
whose cause is gone moves to `close`. Write `why` as one sentence a person can check, naming the cost and
the fix, such as *Six Codex bump runs unverified; rotate one credential*.

The ranking is a proposal. A person can approve any cluster, ranked or not.

## 7. Publish the page

One page per run, under the slug `feedback-triage-<YYYY-MM-DD>` in UTC. It
lists the clusters with a member in this run's window.

```bash
cp <skill>/scripts/page.html page.html
moat artifact publish page.html --title "Feedback triage $(date -u +%F)" \
  --slug "feedback-triage-$(date -u +%F)" --data --rules <skill>/scripts/rules.json --private
```

Then write the records, `meta/run` first:

```bash
moat artifact data save "$slug" meta/run "$(jq -c . meta.json)"
for f in clusters/*.json; do moat artifact data save "$slug" "clusters/$(basename "$f" .json)" "$(jq -c . "$f")"; done
```

`meta/run` holds `{runDate, windowDays, processed, windowFrom, windowTo,
clusters, openTotal}`. The page renders from the
records, so a record you correct shows up without republishing.

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
  publishing without them. The rules carry no `notify`: this skill acts on no
  decision, so a message per decision would wake the Loop for nothing.
- **Read who decided from a decision's `submittedBy`.**

  The page appends each reader's first decision under a server-made id and
  updates that record afterwards, so a reader cannot claim the key another
  reader would save under. Two open tabs can still give one reader two
  records for a cluster: take that reader's newest by `updatedAt`.
  Only a Moatless admin's decision will count: a user whose `role` is `admin`
  in `moat user list --json`.

## 8. Record the run

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

`triage:runs` keeps the newest 60 runs, which stays well under 16 KiB. A page
lives on the task that published it, so reading its decisions later takes
`moat artifact data list <pageSlug> --task <taskId>`.

End with one reply: the page link, how many items and clusters, the ranked
clusters with their owners, and how many clusters to close.

## Done when

- The page is published with `--data --rules … --private`, and every
  cluster on it has a record.
- Each cluster names an owner, and an owner that differs from every
  `reportedFrom` carries an `ownerReason`.
- Each cluster has a `bucket`, at most ten have `bucket: "fix"`, and each of
  those has a `rank`, a `score` and a `why`.
- Every cluster on the page, then the run, is in loop memory.
- No task was created and no feedback status changed.
