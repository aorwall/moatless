---
name: orchestrate
description: "End-to-end feature orchestration: grill requirements, write spec, render HTML for review, break into tickets, spawn parallel child tasks, review PRs, loop until done. Use when a user wants to plan and execute a non-trivial feature across many tasks."
disable-model-invocation: true
---

# Orchestrate

Drive a feature from idea to merged PRs: clarify → spec → review → tickets → spawn → monitor → code-review → loop.

This skill spawns child Moatless tasks to implement work in parallel. Each child creates a PR and reports back; you run code review on each and advance blocked tickets as their dependencies land.

Three artifacts are produced along the way, each with a distinct audience, and they do not all exist at once:

1. **HTML page** — `/opt/moatless/workspace/scratchpad/<feature-slug>.html`. One file at one URL, first rendered when the research lands and live until the breakdown is agreed. While that is happening it is **the only document**: the only thing the user is pointed at, and the only record of what has been settled. Every answer is recorded on it before the next question goes out; it is redrawn only when an answer made something already on it wrong. It never carries the question being asked — that is the chat's job.
2. **Spec markdown** — `specs/<feature-slug>.md` in the repo. Written **once, after the plan and the breakdown are both agreed**, complete and uncapped, for the child agents. Inlined into every child's spawn message.
3. **Ticket markdown files** — `/opt/moatless/workspace/plans/<feature-slug>/<NN>-<slug>.md` — one per ticket; each becomes the body of its child's spawn message.

Nothing is drafted in `specs/` while the user is still deciding. A half-written spec is a second place for an answer to live, and the two disagree within a few questions.

---

## Phase 1 — Alignment

Run the `/writing-specs` skill and follow its sequence. **Research first:** read the affected code, work out how it behaves today, and render the page from it before asking anything — at feature size this is what stops the interview turning into an explanation session, because every later question can point at the page instead of carrying its own background. Give the URL once.

Then follow its grill: open with the understanding batch (does the research match their model, and do the terms agree with `CONTEXT.md`), then scope, then decisions. One question per turn, each carrying your recommended answer and 2–4 ranked alternatives, each restating position and settled state. Name each batch and its size as it opens, and offer the whole batch at once the first time — a feature-sized interview is long enough that the shape should be the user's choice, not the skill's. Look codebase facts up rather than asking. Record load-bearing decisions as ADRs in `docs/adr/` as they land, and sharpen terms into `CONTEXT.md` as they settle.

Record each answer on the page as it lands — one `Edit`, before the next question — and redraw only when an answer made something already on it wrong. Nothing goes into `specs/` yet. The question stays in chat.

Stop when every section of the page is filled or explicitly parked into Open questions.

---

## Phase 2 — Agreement

Still no spec file. Everything the user is agreeing to lives on the page.

Sketch the testing seams (fewest possible; prefer the highest seam). Confirm them with the user, and record the answer on the page.

Draft the ticket breakdown now, before asking for approval — the work graph is part of what the user approves, and a plan agreed without it gets re-opened as soon as the shape of the work becomes visible.

Paste the decision table into the message alongside the URL, so the choice is reachable without opening anything. **Wait for approval before continuing.** If they want changes, update the page and revise the draft breakdown. Iterate until they approve both the plan and the breakdown.

---

## Phase 3 — Spec and tickets

The user has approved the plan and the proposed breakdown. Now the markdown gets written, in this order.

**First, the spec.** Write **`specs/<feature-slug>.md`** in one pass, in the shape `/writing-specs` defines — fixed heading order, nothing capped, every decision carrying the alternatives it beat. Use the domain glossary (`CONTEXT.md`) and existing ADRs throughout. This file is the child tasks' only view of the feature: the page is not in the repo, they cannot read it, and anything agreed there and not written here is lost.

**Then the tickets.** One file per ticket to **`/opt/moatless/workspace/plans/<feature-slug>/<NN>-<slug>.md`**, numbered from `01` in dependency order.

<ticket-template>
# <NN> — <Ticket title>

## What to build

End-to-end behaviour this ticket makes work, from the user's perspective.

## Spec reference

Feature: <feature-slug> — `specs/<feature-slug>.md`

## Blocked by

Ticket numbers/titles that must complete before this one can start, or "None — can start immediately".

## Acceptance criteria

- [ ] Criterion 1
- [ ] Criterion 2

## Status

ready-for-agent
</ticket-template>

In each ticket's Spec reference section, note that the full spec is included in the child's spawn message — children cannot read the orchestrator's files (see Phase 4).

Present the breakdown as a numbered list (title / blocked-by / what it delivers). Ask whether granularity is right and whether edges are correct. Iterate until approved. Once approved, update the spec's `## Ticket breakdown` graph to match, then redraw the page with per-ticket cards below its existing sections, so the page the user has open matches the final plan.

---

## Phase 4 — Spawn

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

## Phase 5 — Monitor and Review

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

## Phase 6 — Summary

Report to the user:

- All tickets: title, PR URL, review outcome
- Any open items or follow-up work
- Suggested next steps (merging, deployment, archiving the spec and plan files)
