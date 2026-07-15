---
name: sandbox-troubleshooting
description: >-
  Troubleshoot a Moatless task — detect what's wrong with its sandbox, agent,
  or servers, whether the task is stuck, unresponsive, restarted, evicted,
  OOMKilled, won't start, or a preview server is failing. A detection-first
  toolbox (moat CLI diagnose/status/events/messages/logs + kubectl) plus a
  symptom decision tree. Triggers on: task stuck, sandbox restarted, pod evicted,
  OOMKilled, CrashLoopBackOff, sandbox won't start, stuck initializing, agent not
  responding, preview server down, investigate sandbox/task failure,
  troubleshoot task, sandbox troubleshooting.
---

# Task & Sandbox Troubleshooting

Detect and diagnose what's wrong with a Moatless task: its sandbox (one
Kubernetes Deployment per task), the agent, the run itself, or a server
container. This skill is a **detection-first toolbox** — start broad with the
`moat` CLI, then narrow to a specific cause. Symptom-specific playbooks live in
[references/decision-tree.md](references/decision-tree.md).

## Golden rule: CLI first, then kubectl, then code

Diagnose in this order:

1. **`moat` CLI** — works everywhere (even with no cluster creds), gives a
   one-shot aggregate (`moat tasks diagnose`), and reads the **durable** event
   history that outlives the pod. Start here.
2. **`kubectl`** — when you have cluster access and need live pod internals.
   From inside a sandbox the `moatless-sandbox` SA can now read events + pod
   metrics too (`kubectl get events`, `kubectl top pods`) via the
   `-observability` RoleBinding shipped in Helm chart ≥ 0.1.1; `get pvc` /
   role listing / deploy delete still need admin. See
   [references/kubectl.md](references/kubectl.md).
3. **Source code** — to interpret a signal or find a classification bug.

Why CLI-first even though kubectl now sees events: the event stream is
Postgres/Redis-backed, so `moat events list` still works **after the pod is
gone** (live `kubectl logs --previous` returns 503 once the pod is deleted), and
`moat tasks diagnose` bundles container diagnostics + runtime events + recent
errors in a single call.

> One-line triage: `moat tasks diagnose <task_id>` — aggregates status,
> per-container diagnostics (restarts, exit codes, OOM), K8s runtime events, and
> recent error events in a single call.

## Step 0 — Identify the task, then the pod

The UI shows an 8-char **shortId**. Every `moat tasks …` command accepts either
the shortId or the full **task_id** (UUID) — the CLI resolves a shortId against
the 200 most recent tasks. `kubectl`, however, needs the full UUID (it's in the
deployment/pod names), so grab it once:

```bash
NS=moatless                     # sandbox namespace (this cluster)
# moat commands take the shortId directly:
moat tasks diagnose <shortId>
# Need the full UUID for kubectl? `moat tasks get` now prints it (Task + ShortId lines):
TASK_ID=$(moat tasks get <shortId> --json | jq -r '.taskId')

# Deployment/Service name is DETERMINISTIC: moatless-ws-{task_id} (sanitized, <=63 chars)
echo "moatless-ws-$TASK_ID"

# Pod name is NOT deterministic (random rs/pod hash suffixes) — always look it up by label.
# Filter to the Running pod so you don't grab a terminating one during a rollout.
kubectl get pod -n $NS -l moatless.io/task-id=$TASK_ID \
  --field-selector=status.phase=Running -o jsonpath='{.items[0].metadata.name}'
```

Canonical labels on every sandbox pod: `moatless.io/task-id=<task_id>`,
`app.kubernetes.io/name=moatless-sandbox`, `app.kubernetes.io/component=sandbox`,
`app.kubernetes.io/managed-by=moatless`. Full derivation details, including the
sanitize rules and the rolling-update two-pod gotcha, are in
[references/kubectl.md](references/kubectl.md).

## Detecting issues

Cast a wide net first — these are the detection moves, roughly cheapest/broadest
to most specific. You don't need all of them; pick by what you know. Each says
**what it tells you**.

| Move | What it tells you |
|---|---|
| `moat tasks diagnose <task_id>` | One-shot health: env status, per-container diagnostics (restarts, exit codes, **OOM**), K8s runtime events, recent error events. **Best first call.** |
| `moat tasks status <task_id>` | Current environment status + agent state + per-server status/port/error. Is it Ready / Initializing / Stopped / Error? Is the agent idle, running, or unreachable? |
| `moat events list <task_id> --errors` | Just the error events across the task's life — fastest way to see *what already went wrong* (sandbox/agent/server). Add `--types sandbox,server,agent` to scope, `--json` to filter. |
| `moat messages list <task_id> --full` | The **run transcript** — what the agent actually did, the last thing before it stalled, tool errors, a permission prompt it's blocked on, or (for OOM) how many subagents/builds were running. See below. |
| `moat tasks logs <task_id> --server sandbox [--previous]` | Container stdout/stderr. `--previous` = the crashed instance. Servers: `sandbox`, `preview`, `files`, `<name>-server`. |
| `kubectl get pod -l moatless.io/task-id=<id> -o wide` + `kubectl get events` / `top pods` | Live pod state, restart counts, node, K8s events, resource usage (see [references/kubectl.md](references/kubectl.md)). |
| `GET /metrics` on the backend | Fleet-wide signal: `sandbox_container_terminations_total`, `task_reconcile_remediations_total`, startup latency (see [references/commands.md](references/commands.md)). |

> Rule of thumb: `moat tasks diagnose` tells you the **current** state;
> `moat events list --json` + `moat messages list --full` tell you the
> **history** — you almost always need history to explain a restart.

### Reading the run transcript (`moat messages`)

Much of "why is this task stuck/broken" lives in the conversation, not the
infra. `moat messages` reads it:

```bash
moat messages list <task_id> --full             # full transcript: roles, timestamps, tool markers
moat messages list <task_id>                    # compact preview table (newest 50)
moat messages list <task_id> --limit 200        # bigger page (max 200)
moat messages list <task_id> --before-id <id>   # page further back (cursor from previous page)
moat messages list <task_id> --json | jq ...    # programmatic filtering
```

Use it to: spot the last action before a stall or OOM, find a tool/permission
error the agent reported, confirm whether the agent even received a message, or
correlate a spike in parallel work with a `container_oom_killed` event. Pair it
with `moat events list` — infra events tell you *when* something broke; the
transcript tells you *what the agent was doing* at that moment.

### Then: narrow to the cause

Once a symptom is visible, follow the matching branch in
[references/decision-tree.md](references/decision-tree.md) (detect → confirm →
cause → fix for restarts, won't-start, OOM, eviction, server failure, agent
stuck, idle-stop). The symptom→cause→fix lookup table and all tuning knobs are
in [references/failure-modes.md](references/failure-modes.md).

## Escalate to code

When a signal is ambiguous or you suspect a classification bug, jump to the
source. The symptom→file index is in [references/code-map.md](references/code-map.md).
Start with `docs/sandbox-lifecycle.md` (canonical state machine) and
`backend/src/sandbox/k8s/pod_watcher.rs` (failure/eviction classification).

## Reference files

- [references/decision-tree.md](references/decision-tree.md) — symptom → detect → confirm → cause → fix branches.
- [references/commands.md](references/commands.md) — copy-paste diagnostic blocks (moat CLI + kubectl + metrics).
- [references/failure-modes.md](references/failure-modes.md) — symptom → event/reason → cause → fix table; all size/env knobs.
- [references/kubectl.md](references/kubectl.md) — tested RBAC matrix, pod-name derivation, jsonpath cookbook, the events/top-pods RBAC grant.
- [references/code-map.md](references/code-map.md) — symptom → source-file index, event types, metrics.
