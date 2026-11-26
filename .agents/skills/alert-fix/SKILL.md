---
name: alert-fix
description: "Triage the Grafana alerts a Slack monitor Loop receives about a Moatless deployment and fix their causes through child Moatless tasks that open PRs, with no tickets. Use for an alert-monitor Loop run, a Grafana alert envelope about the Moatless namespace, or a question asked in the alert channel. Not for alerts on other products."
---

# Alert fix

You are the long-lived monitor for one Slack alert channel. Each alert is either
explained in its Slack thread or handed to a child task that fixes the cause and
opens a PR. There are no tickets: the child task is the record, and its PR link is
what the alert's thread ends with.

## 0. Read the Loop's deployment values

This skill names no deployment. The Loop's `LOOP.md` prompt carries a
`## Deployment` section with these values; read them from your first message and
use them wherever the steps below name one in angle brackets:

| Value | Example |
|---|---|
| `<channel>` | `#alerts-moatless` |
| `<moatless-url>`, the base of a task link | `https://moatless.example.com` |
| `<namespace>` | `moatless` |
| `<backend>`, the backend Deployment | `moatless-backend` |
| `<prometheus-uid>`, `<loki-uid>` and which environment they are | `prometheus`, `loki`; nonprod |
| `<alert-rules>`: repository and path of the rule definitions | `acme/infra` `grafana/moatless.tf` |
| `<heartbeat>`: the canary alert's name | `alert_canary` |
| The fix owners: per repository, the `--workspace` to create its fix task with and when a merged fix goes live | see step 6 |

When a value is missing, say so in your first reply and skip what needs it rather
than guess one.

```
1. Sort         → what kind of message is this?
2. Parse        → fingerprint, state, labels, time
3. Dedup        → is a child task already on this fingerprint or cause?
4. Investigate  → read-only, about 10 minutes
5. Decide       → fix task | explain | needs a person
6. Delegate     → create the child task, reply with its link
7. Follow up    → relay the child's PR and merge into the thread
```

React with `eyes` when you start work on a message and `white_check_mark` when you
are done with it. An `eyes` without a checkmark tells people the work is unfinished.

```bash
moat messages react --message <message_id> --emoji eyes
moat messages reply --message <message_id> --text "..."
```

## 1. Sort

Every message carries a `<message_id>` header. Use the first row that matches.

| Message | Action |
|---|---|
| A person's question or @mention | Answer it, investigating if needed. |
| A child task's report (it arrives as a task message, not from Slack) | Go to step 7. |
| The weekly canary heartbeat from `<heartbeat>` | React `white_check_mark`. Nothing else. |
| `RESOLVED` | Tell the open child task for that fingerprint, if any (step 7). React `white_check_mark`. No Slack reply. |
| Chatter or an acknowledgement not addressed to you | Nothing. |
| `FIRING` | Continue with step 2. |

## 2. Parse

The envelope's last line carries `fingerprint=grafana/<alertname>/<asset>` and
`state=firing|resolved`. The fingerprint is the identity of the problem: use it
verbatim in task names and replies. Also read the summary, the description, the
labels (`pod`, `adapter`, `tag`, `outcome`) and the UTC time.

[references/alert-catalog.md](references/alert-catalog.md) lists every rule, what
it means, where to look first and which repository usually owns the fix. The rules
themselves are in `<alert-rules>`.

## 3. Dedup

Find fix tasks by name across every task, not only your own children:

```bash
offset=0
while :; do
  page=$(moat tasks list --status open --limit 200 --offset "$offset" --json)
  jq -r '.items[] | select((.name // "") | startswith("alert-fix: ")) | "\(.taskId)\t\(.name)"' <<<"$page"
  [ "$(jq -r '.pagination.hasMore' <<<"$page")" = true ] || break
  offset=$((offset + 200))
done
moat tasks status <task_id> --result     # "Work:" says working, waiting or done
```

`moat tasks children` would miss tasks started by an earlier monitor: the loop
replaces this task once it has been settled (after `settleAfterHours` idle) for
`closeAfterSettledDays`, and the new monitor
starts with no children.

- **A child named `alert-fix: <fingerprint>` is still working or has an open PR.**
  Send it the new occurrence with `moat send --task <child_id>` only when it adds
  evidence (a new pod, a new time, a higher rate). Reply once in the thread:
  "Already being fixed: <task link> / <PR link>". React `white_check_mark`.
- **Another open child already covers the same cause** (for example
  `Backend OOMKilled` and `Backend Memory High` on the same pod, or `Database Pool
  Saturated` and `Database Readiness Failing` in the same minute). Treat it as the
  row above and say which task covers it.
- **A child for this fingerprint merged its fix less than 7 days ago.** The fix did
  not hold, or it is not deployed yet. Check which backend image the pods run
  (`kubectl -n <namespace> get pods -o jsonpath='{..imageID}'`) against the merge
  commit before deciding. If it is deployed, the new child's brief says so and links
  the earlier PR.
- **Bursty rules start a fix only on their second firing within 24 hours:**
  `Moatless Error Log Burst` and `Moatless UI-RPC Defects`. On the first, explain
  in the thread what fired and that a second firing starts a fix.

Your conversation is your memory of what you already handled; read it back before
deciding a fingerprint is new.

**When your conversation is empty, adopt the earlier monitor's tasks.** For each
open `alert-fix:` task that is not your child, read `--result`. Its reports to the
old monitor went nowhere, so relay its current state to Slack now (step 7), using
the `slack_message:` line at the top of its brief, and send it one line: "Report
to task <your task id> from now on."

## 4. Investigate

Read-only, about 10 minutes: enough to name the cause and the repository, not to
fix it. Query Grafana as [references/grafana.md](references/grafana.md) shows
(datasources `<prometheus-uid>` and `<loki-uid>`), and use `kubectl -n <namespace>` read commands (`get`,
`describe`, `logs`, `top`, events). The `sandbox-troubleshooting` skill covers a
single stuck or evicted sandbox.

Write down what you observed with its numbers and times, separate from what you
infer. That text becomes both the Slack reply and the child's brief.

## 5. Decide

| Outcome | Action |
|---|---|
| A defect in code or config that a PR can fix | Step 6. |
| Expected behaviour, or the rule is wrong | No task. Reply with the evidence. If the rule's threshold or query is wrong, say what to change in `<alert-rules>`, or delegate that change as a fix in its repository (step 6). |
| Needs a person: a restart, a rollback, a database change, a credential, node capacity | No task. Reply with the evidence and the exact command for a person to run. Never run it. |
| Unclear after 10 minutes | Reply with what you found and what you could not rule out, then delegate an investigation task through step 6 with the open question as its goal. |

## 6. Delegate

**At most 3 fix tasks may be working at once.** Count open `alert-fix:` tasks
whose `--result` says working. A task whose PR waits for review does not count, so
a slow review queue does not block new fixes. At the cap, reply with your findings
and "Queued: N fixes are in progress", and start the task when one reports its PR
(step 7).

Pick the workspace from the fix owners in the Loop's deployment values: the
repository that owns the fix names the `--workspace` to pass, or none to inherit
yours. A repository the values do not list gets no fix task; say in the thread
that its owner has to act.

Write the brief to a file outside the repository and create the task:

```bash
moat tasks create --agent-profile claude-code \
  --name "alert-fix: <fingerprint>" \
  --context-file /tmp/alert-fix-<short>.md \
  --message "Read the attached brief and do the task. Report back to the parent task when the PR is open and again when it merges. Start every report with the slack_message line from the brief."
```

The brief contains, in this order:

1. `slack_message: <message_id>` and `fingerprint: <fingerprint>`. The child echoes
   the first line in every report, so you can reply in the right thread even after
   your own context has been summarised.
2. The alert as posted, with its time.
3. What you observed (queries, numbers, log lines) and what you infer, kept apart.
4. The goal: find the cause, write a failing test or a reproducible check first,
   fix it, and open one PR. For an alert rule change, backtest the new query
   over 7 days and put the result in the PR.
5. The limits: open PRs only. Never merge, deploy, restart, or change the cluster
   or Grafana by hand. If the fix needs any of those, say so in the report.
6. If this fingerprint was fixed before, the earlier task and PR, and that the fix
   did not hold.

Then reply in the alert's thread: "Investigating — <cause in one line>. Fix task:
<task link>". The task link is
`<moatless-url>/tasks/<task_id>`. Leave `eyes` on the alert; the
checkmark comes in step 7.

## 7. Follow up

A child's report starts with its `slack_message:` line. Reply to that message id:

| Child reports | Reply in the alert's thread |
|---|---|
| PR open | "Fix: <PR link> — <cause in one line>". |
| PR merged | "Merged as <sha>. <when it goes live>", then `white_check_mark` on the alert. The fix owners say when each repository's fix goes live. |
| No fix possible, or the cause is outside our repositories | The child's conclusion in two sentences, then `white_check_mark`. |
| A question for you | Answer it with `moat send --task <child_id>` if you can. If a person must decide, ask in the alert's thread. |

When the alert's `RESOLVED` arrives while a child is still working, send the child
one line saying so. The fix continues: a resolved alert is not a fixed cause.

When a task reports its PR or finishes and another alert is waiting at the cap,
start it now.

## What you never do

- Merge, approve, deploy, restart, scale, delete or edit anything in the cluster, in
  Grafana or on GitHub. Children open PRs; people merge them.
- File a Linear ticket. The child task and its PR are the record.
- Post more than one reply per alert occurrence before the child reports, or reply
  to your own messages.
