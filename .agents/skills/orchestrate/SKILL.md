---
name: orchestrate
description: "End-to-end feature orchestration: grill requirements, write spec, render HTML for review, break into tickets, spawn parallel child tasks, review PRs, loop until done. Use when a user wants to plan and execute a non-trivial feature across many tasks."
disable-model-invocation: true
---

# Orchestrate

Drive a feature from idea to merged PRs: clarify → spec → review → tickets → spawn → monitor → code-review → loop.

This skill spawns child Moatless tasks to implement work in parallel. Each child creates a PR and reports back; you run code review on each and advance blocked tickets as their dependencies land.

Three artifacts, each with a distinct audience. `/writing-specs` owns when the first two appear; this skill owns the third.

| Artifact | Path | Lives where | Read by |
|---|---|---|---|
| The page | `/opt/moatless/workspace/scratchpad/<feature-slug>.html` | Orchestrator sandbox | The user, until the breakdown is agreed |
| The spec | `<repo>/.plans/<feature-slug>.md` | **Committed to the feature branch** | The child agents — they read it from their own checkout |
| Ticket files | `/opt/moatless/workspace/tickets/<feature-slug>/<NN>-<slug>.md` | Orchestrator sandbox | One per child, inlined as its spawn message body |

The spec is committed and the tickets are not, and the difference is
deliberate. A child needs the spec at any moment, so it goes where every
checkout already has it. A child needs its own ticket exactly once, at spawn,
so the spawn message is a sufficient copy — and keeping ticket churn out of the
branch keeps the feature's diff about the feature.

---

## Phase 1 — Plan and break down

Run **`/writing-specs`** and follow its workflow to the written spec, then **`/to-tickets`**. Both skills own their own approval gate; don't add a third.

Four things differ at feature size:

1. **The spec is the child tasks' only view.** They run in fresh sandboxes and cannot read the page, the ticket files, or anything else in your sandbox. Whatever is agreed on the page and not written into the spec is lost.
2. **The spec is committed to a feature branch before anything is spawned.** Write it to `<repo>/.plans/<feature-slug>.md`, commit, and push:

   ```bash
   git checkout -b <feature-slug> origin/main
   mkdir -p .plans && cp <draft> .plans/<feature-slug>.md
   git add .plans/<feature-slug>.md && git commit -m "docs: spec <feature-slug>" && git push -u origin <feature-slug>
   ```

   That branch is the root of the whole stack: every ticket branch descends from it, and it becomes the integration PR at the end.
3. **Write ticket files to `/opt/moatless/workspace/tickets/<feature-slug>/<NN>-<slug>.md`**, numbered from `01` in dependency order — that is this skill's tracker. They stay in your sandbox; each one reaches its child by being inlined into the spawn message (see Phase 2).
4. **Once the breakdown is approved, redraw the page with per-ticket cards** below its existing sections, so the page the user has open matches what is about to be spawned.

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

**Inline the ticket; point at the spec.** A child cannot see your
`/opt/moatless/workspace/tickets/` or `scratchpad/` files, so its ticket must be
inlined. The spec is different — `--branch` checks the child out on a branch
that already contains `.plans/<slug>.md`, so it reads the live file instead of a
frozen copy. Give it the path, not the body.

For each unblocked ticket:

```bash
CHILD_ID=$(moat tasks create \
  --workspace <ws-id> \
  --parent $TASK_ID \
  --name "Ticket <NN>: <slug>" \
  --branch <BASE_BRANCH> \
  --message "$(cat /opt/moatless/workspace/tickets/<slug>/<NN>-<file>.md)

---
ORCHESTRATOR_TASK_ID: $TASK_ID
BASE_BRANCH: <branch>       # what --branch put you on; branch from it, target your PR at it
SPEC: .plans/<slug>.md      # in your checkout, already current — read it, do not ask for it

Use /implement-ticket to implement this work. Create a PR when done.
" --json | jq -r '.taskId')
echo "Spawned $CHILD_ID for ticket <NN>"
```

(The create response's field is `taskId`, not `id`.)

**You own the base branch; children never reconcile the spec.** A child reads
the spec once, from the branch you started it on, and that branch does not move
under it. Making sure it is the right spec is your job, done *before* the
`moat tasks create` call:

- A ticket stacked on a dependency inherits any spec amendment that dependency
  made, for free — that is the common case and needs nothing from you.
- A ticket spawned onto some other branch does not. If a landed PR amended the
  spec and this child needs the amendment, merge it into that branch and push
  **first**, then spawn.
- Children spawned in the same wave cannot see each other's amendments. Accept
  that: reconcile at review time, when you are reading both PRs anyway.

Never ask a child to rebase or re-pull to pick up a spec change. If it needs a
different spec than the one it started with, tell it what changed in a
`moat send`, or close it and re-spawn from a corrected branch.

**Every ticket ships as its own PR** — one ticket, one branch, one PR. An unblocked ticket bases on the feature branch (`--branch <feature-slug>`), never on the default branch: that is where the spec is. Dependent tickets don't wait for their blocker's PR to merge: **stack them**. When ticket B is blocked by ticket A and A's PR has passed review, spawn B with `--branch <A's PR branch>` — the child branches from it and opens its PR with that base. Record each child's PR branch (from its report-back) so you can stack dependents on it. Stacked PRs merge in dependency order; after a base PR merges, ask the dependent's child to rebase onto the branch below it (GitHub retargets the PR base automatically on merge).

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
   - Identify the spec at `.plans/<slug>.md` on the PR's branch — the child may
     have amended it; that amendment is part of what you are reviewing
   - Run Standards + Spec review in parallel

2. If review finds real issues, send feedback to the child:
   ```bash
   moat send --task <child-id> --text "Code review feedback for your PR:

   <findings>

   Please fix these and push an update, then message me again when ready."
   ```

3. If review passes (or after fixes are applied), mark the ticket complete and check whether any blocked tickets are now fully unblocked. Spawn those tickets immediately, stacked on the reviewed PR's branch (`--branch <that branch>`) — do not wait for the PR to merge. Stacking is also how the next child inherits this one's spec amendment; if you spawn it anywhere else, merge the amendment into that base first.

**When a child reports "Blocked on: <description>":**

Send a clarification or resolve the blocker, then let the child continue or close and re-spawn as needed.

Repeat until all tickets are complete and all PRs have passed review.

---

## Phase 4 — Land and archive

Once every ticket's PR has passed review and the stack has merged down to the
feature branch, that branch is the feature's PR against the default branch.

Before opening it, retire the spec:

```bash
git mv .plans/<feature-slug>.md .plans/completed/<feature-slug>.md
git commit -m "docs: complete <feature-slug>"
```

`.plans/` is therefore a list of what is in flight, and `.plans/completed/` the
record of what shipped — with the final spec text, not the version anyone was
spawned against. Do this **last**: the moment it moves, no further child can be
spawned against `.plans/<slug>.md`. If a follow-up ticket is still open, leave
the spec where it is.

The ticket files under `/opt/moatless/workspace/tickets/` are scratch and are
not archived. Anything in them worth keeping belongs in the spec.

Report to the user:

- All tickets: title, PR URL, review outcome
- Any open items or follow-up work
- Suggested next steps (merging the feature branch, deployment)
