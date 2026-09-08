---
name: orchestrate
description: "End-to-end feature orchestration: grill requirements, write spec, render HTML for review, break into tickets, spawn parallel child tasks, review PRs, loop until done. Use when a user wants to plan and execute a non-trivial feature across many tasks."
disable-model-invocation: true
---

# Orchestrate

Drive a feature from idea to merged PRs: clarify → spec → review → tickets → spawn → monitor → code-review → loop.

This skill spawns child Moatless tasks to implement work in parallel. Each child creates a PR and reports back; you run code review on each and advance blocked tickets as their dependencies land.

Three artifacts, each with a distinct audience. `/spec` owns when the first two appear; this skill owns the third.

| Artifact | Path | Lives where | Read by |
|---|---|---|---|
| The page | `/opt/moatless/workspace/scratchpad/<feature-slug>.html` | Orchestrator sandbox | The user, until the breakdown is agreed |
| The spec | `<repo>/.plans/<feature-slug>.md` | **Committed to the stack root branch** | The child agents — they read it from their own checkout |
| Ticket files | `/opt/moatless/workspace/tickets/<feature-slug>/<NN>-<slug>.md` | Orchestrator sandbox | One per child, inlined as its spawn message body |

The spec is committed and the tickets are not, and the difference is
deliberate. A child needs the spec at any moment, so it goes where every
checkout already has it. A child needs its own ticket exactly once, at spawn,
so the spawn message is a sufficient copy — and keeping ticket churn out of the
branch keeps the feature's diff about the feature.

---

## Phase 1 — Plan and break down

Run **`/spec`** and follow its workflow to the written spec, then **`/to-tickets`**. Both skills own their own approval gate; don't add a third.

Four things differ at feature size:

1. **The spec is the child tasks' only view.** They run in fresh sandboxes and cannot read the page, the ticket files, or anything else in your sandbox. Whatever is agreed on the page and not written into the spec is lost.
2. **The approved spec is the first layer of the GitHub stack.** Write it to
   `<repo>/.plans/<feature-slug>.md`, commit it on a spec branch, initialize a
   stack rooted at the default branch, and submit that branch before any child
   is spawned:

   ```bash
   SPEC_BRANCH=<feature-slug>-spec
   git checkout -b "$SPEC_BRANCH" origin/main
   mkdir -p .plans && cp <draft> .plans/<feature-slug>.md
   git add .plans/<feature-slug>.md
   git commit -m "docs: spec <feature-slug>"
   moat gh stack init --base main "$SPEC_BRANCH"
   moat gh stack submit --auto --open --task "$TASK_ID"
   moat gh stack view --json
   ```

   Record the spec branch and spec PR URL. That PR is the bottom of the whole
   feature stack; every ticket branch descends from it. Use `--auto` for
   non-interactive submission, and keep `--open` only when the approved spec is
   ready for review.
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

Pass `--workspace <ws-id-or-name>` to `moat tasks create`. It is required: a
task is always created in a Workspace, and `--repo` no longer names one.

Identify which tickets have no blockers (or whose blockers are already complete), then assign stack order before spawning. A GitHub stack is linear: a child can `moat gh stack add` only on top of an existing top branch. Spawn every unblocked ticket whose `STACK_BASE_BRANCH` already exists; in a single feature stack this often means spawning the next ticket as soon as the previous child has created its branch or PR. Preserve parallelism only when you intentionally create separate stacks or pre-created placeholder stack branches for independent work.

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
  --branch <STACK_BASE_BRANCH> \
  --message "$(cat /opt/moatless/workspace/tickets/<slug>/<NN>-<file>.md)

---
ORCHESTRATOR_TASK_ID: $TASK_ID
SPEC: .plans/<slug>.md              # in your checkout, already current — read it, do not ask for it
STACK_ROOT_BRANCH: <spec-branch>    # bottom branch of the feature stack
STACK_BASE_BRANCH: <branch>         # what --branch put you on; add your ticket branch on top of it
BASE_BRANCH: <branch>               # compatibility alias for STACK_BASE_BRANCH
STACK_BASE_PR: <pr-url-or-number>   # PR for STACK_BASE_BRANCH; use it to recover local stack tracking
STACK_BRANCH: ticket-<NN>-<slug>    # branch name to create with moat gh stack add

Use /implement-ticket to implement this work. Because this ticket is stacked,
its PR creation step is: recover the stack, run 'moat gh stack add
<STACK_BRANCH>' before coding, then run 'moat gh stack submit --auto --task
$TASK_ID' when done. Do not use 'moat gh pr create' for this ticket.
" --json | jq -r '.taskId')
echo "Spawned $CHILD_ID for ticket <NN>"
```

(The create response's field is `taskId`, not `id`.)

**You own the base branch and spec reconciliation.** A child reads the spec
once, from the branch you started it on, and that branch does not move under
it. When a child reports a finding that changes the spec or ticket plan,
decide the change, amend the appropriate stack branch yourself, and spawn later
children from a branch that contains it. Making sure a child starts from the
right spec is your job, done *before* the `moat tasks create` call:

- A ticket stacked on a dependency inherits that dependency's code and any
  orchestrator-approved spec update already committed below it.
- A ticket spawned onto some other branch does not. If a reviewed finding
  changed the spec and this child needs the change, merge or cherry-pick that
  update into the child's base branch and push **first**, then spawn.
- Children spawned in the same wave cannot see each other's findings. Accept
  that: reconcile at review time, when you are reading both PRs anyway.

Never ask a child to rebase or re-pull to pick up a spec change. If it needs a
different spec than the one it started with, tell it what changed in a
`moat send`, or close it and re-spawn from a corrected branch.

**Every ticket ships as its own stack layer** — one ticket, one branch, one PR.
The first ticket stacks on the spec PR branch. Each later ticket stacks on the
reviewed branch below it, not on the default branch. Children create their own
layer with `moat gh stack add <STACK_BRANCH>` and submit with
`moat gh stack submit --auto --task $TASK_ID`, which pushes all active stack
branches, creates or updates PRs, and links the GitHub stack. Record each
child's branch and PR URL from its report-back so the next child receives the
right `STACK_BASE_BRANCH` and `STACK_BASE_PR`.

Use these stack commands deliberately:

- `moat gh stack init --base main <spec-branch>` once, after spec approval, to
  make the spec branch the stack root.
- `moat gh stack checkout <STACK_BASE_PR>` in child tasks when local stack
  tracking is missing.
- `moat gh stack add <STACK_BRANCH>` in child tasks before implementation.
- `moat gh stack submit --auto --task $TASK_ID` after each stack layer is ready;
  add `--open` only when the PRs should be marked ready for review.
- `moat gh stack view --json` whenever you need the current branch/PR order.
- `moat gh stack sync --prune` after remote stack changes or merges, and
  `moat gh stack rebase` when the stack must be replayed after trunk moves or a
  conflict is reported.

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
   - Identify the spec at `.plans/<slug>.md` on the PR's branch. If the child
     reported a finding, review it against the orchestrator-approved spec state
     for that stack layer.
   - Run Standards + Spec review in parallel

2. If review finds real issues, send feedback to the child:
   ```bash
   moat send --task <child-id> --text "Code review feedback for your PR:

   <findings>

   Please fix these and push an update, then message me again when ready."
   ```

3. If review passes (or after fixes are applied), mark the ticket complete and check whether any blocked tickets are now fully unblocked. Spawn the next stack layer on the reviewed PR's branch (`--branch <that branch>`) as soon as that branch exists — do not wait for the PR to merge. Stacking is also how the next child inherits any orchestrator-approved spec update committed below it; if you spawn it anywhere else, merge that update into the base first.

**When a child reports "Blocked on: <description>":**

Send a clarification or resolve the blocker, then let the child continue or close and re-spawn as needed.

Repeat until all tickets are complete and all PRs have passed review.

---

## Phase 4 — Land and archive

Once every ticket's PR has passed review, use the GitHub stack to land the
feature. `moat gh stack merge` merges the stack in order; use the interactive
flow when a human should choose the method, or `moat gh stack merge --yes
--squash` only when the project convention and branch protection allow it.

The final step after implementation is archiving the completed plan. Do not
leave an implemented feature's spec at `.plans/<feature-slug>.md`.

After the stack lands, retire the spec in a final documentation PR or a cleanup
stack layer:

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
- Completed plan archived at `.plans/completed/<feature-slug>.md`, or why it
  intentionally remains active
- Any open items or follow-up work
- Suggested next steps (merging the feature branch, deployment)
