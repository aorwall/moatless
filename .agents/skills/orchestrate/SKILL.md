---
name: orchestrate
description: "End-to-end feature orchestration: grill requirements, write spec, render HTML for review, break into tickets, spawn parallel child tasks, review PRs, loop until done. Use when a user wants to plan and execute a non-trivial feature across many tasks."
disable-model-invocation: true
---

# Orchestrate

Drive a feature from idea to merged PRs: clarify → spec → review → tickets → spawn → monitor → code-review → loop.

This skill spawns child Moatless tasks to implement work in parallel. Each child creates a PR and reports back; you run code review on each and advance blocked tickets as their dependencies land.

Three artifacts, each with a distinct audience. `/writing-specs` owns when the first two appear; this skill owns the third.

| Artifact | Path | Read by |
|---|---|---|
| The page | `/opt/moatless/workspace/scratchpad/<feature-slug>.html` | The user, until the breakdown is agreed |
| The spec | `specs/<feature-slug>.md` | The child agents — inlined into every spawn message |
| Ticket files | `/opt/moatless/workspace/plans/<feature-slug>/<NN>-<slug>.md` | One per child, as its spawn message body |

---

## Phase 1 — Plan and break down

Run **`/writing-specs`** and follow its workflow to the written spec, then **`/to-tickets`**. Both skills own their own approval gate; don't add a third.

Three things differ at feature size:

1. **The spec is the child tasks' only view.** They run in fresh sandboxes and cannot read the page, the ticket files, or anything else you have. Whatever is agreed on the page and not written into `specs/<feature-slug>.md` is lost.
2. **Write ticket files to `/opt/moatless/workspace/plans/<feature-slug>/<NN>-<slug>.md`**, numbered from `01` in dependency order — that is this skill's tracker. Each ticket's Spec reference notes that the full spec is inlined into its spawn message, because children cannot read the orchestrator's files (see Phase 2).
3. **Once the breakdown is approved, redraw the page with per-ticket cards** below its existing sections, so the page the user has open matches what is about to be spawned.

---

## Phase 2 — Spawn

Pick where each child runs — **workspace first, bare repo as fallback**:

```bash
moat workspaces list --json | jq '.[] | {id, name, source, dockerAccess}'
moat repos list --json | jq '.[] | {id, name, remoteUrl}'   # fallback only
```

Prefer a named workspace over a bare `--repo`: git-synced workspaces (from the
repo's `.moatless/workspaces.json`) carry the run-config a child needs —
resource sizing (memory/storage for heavy builds), servers, and Docker access.
Spawning by `--repo` lands the child on the repo's bare shadow workspace, which
may have none of that.

Choose per ticket — **default to the sandbox-only workspace**:

- **Default**: the sandbox-only workspace (e.g. `Moatless`) — agent container,
  no dev server. Child tasks write code, run tests, and open a PR; almost none
  of them need a running frontend. A sandbox-only pod is a fraction of the size
  of one carrying a dev server, so several fit per node instead of one, and
  children stop queueing behind each other for capacity. Pick this unless the
  ticket gives you a concrete reason not to.
- **Preview needed**: the dev-server workspace (e.g. `Moatless Preview`) only
  when the ticket needs the app actually running — visual/UI verification, or
  acceptance criteria written against a live preview URL.
- **Docker needed**: the docker-enabled workspace (e.g. `Moatless Fullstack`,
  `dockerAccess: true`) when the ticket runs compose-based integration tests,
  builds images, or otherwise needs a Docker daemon. Check the ticket's
  acceptance criteria for compose/E2E test commands.

Sizing follows that choice automatically — each workspace carries its own
resource config, and servers are sized separately from the agent container.

Pass `--workspace <ws-id-or-name>` to `moat tasks create` (it replaces
`--repo`). If your `moat` build predates the flag (`moat tasks create --help`
doesn't list it), fall back to `--repo <repo-id>`.

Identify which tickets have no blockers (or whose blockers are already complete). Spawn **all unblocked tickets in parallel** — one `moat tasks create` per ticket. Capture each child task ID.

**The spawn message must be fully self-contained.** Each child runs in its own sandbox with a fresh checkout of the default branch — it cannot see the orchestrator's uncommitted `specs/<slug>.md`, nor its `/opt/moatless/workspace/plans/` or `scratchpad/` files. Inline everything the child needs: the full ticket AND the full spec. (Paths below are relative to the repo root.)

For each unblocked ticket:

```bash
CHILD_ID=$(moat tasks create \
  --workspace <ws-id> \
  --parent $TASK_ID \
  --name "Ticket <NN>: <slug>" \
  --message "$(cat /opt/moatless/workspace/plans/<slug>/<NN>-<file>.md)

---
# Feature spec (full text — included because you cannot read the orchestrator's files)

$(cat specs/<slug>.md)

---
ORCHESTRATOR_TASK_ID: $TASK_ID
BASE_BRANCH: <branch>   # only for stacked tickets — see below; omit to base on the default branch

Use /implement-ticket to implement this work. Create a PR when done.
" --json | jq -r '.taskId')
echo "Spawned $CHILD_ID for ticket <NN>"
```

(The create response's field is `taskId`, not `id`.)

**Every ticket ships as its own PR** — one ticket, one branch, one PR. Dependent tickets don't wait for their blocker's PR to merge: **stack them**. When ticket B is blocked by ticket A and A's PR has passed review, spawn B with `BASE_BRANCH: <A's PR branch>` — the child branches from it and opens its PR with that base. Record each child's PR branch (from its report-back) so you can stack dependents on it. Stacked PRs merge in dependency order; after a base PR merges, ask the dependent's child to rebase onto the default branch (GitHub retargets the PR base automatically on merge).

Record a mapping of ticket number → child task ID for monitoring.

---

## Phase 3 — Monitor and Review

Messages from child tasks arrive automatically as system reminders in your conversation — no polling needed. When a child runs `moat send --task $TASK_ID`, it appears inline and you are re-invoked to handle it.

To check overall status at any point:

```bash
moat tasks children --json
```

**When a child reports "PR ready: <url>":**

1. Run `/code-review` on the PR branch vs main:
   - Pin the fixed point as `main` (or the base branch)
   - Identify the spec at `specs/<slug>.md`
   - Run Standards + Spec review in parallel

2. If review finds real issues, send feedback to the child:
   ```bash
   moat send --task <child-id> --text "Code review feedback for your PR:

   <findings>

   Please fix these and push an update, then message me again when ready."
   ```

3. If review passes (or after fixes are applied), mark the ticket complete and check whether any blocked tickets are now fully unblocked. Spawn those tickets immediately, stacked on the reviewed PR's branch (`BASE_BRANCH: <branch>` in the spawn message) — do not wait for the PR to merge.

**When a child reports "Blocked on: <description>":**

Send a clarification or resolve the blocker, then let the child continue or close and re-spawn as needed.

Repeat until all tickets are complete and all PRs have passed review.

---

## Phase 4 — Summary

Report to the user:

- All tickets: title, PR URL, review outcome
- Any open items or follow-up work
- Suggested next steps (merging, deployment, archiving the spec and plan files)
