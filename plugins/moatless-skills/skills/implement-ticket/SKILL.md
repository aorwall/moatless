---
name: implement-ticket
description: "Implement a single orchestrated ticket, create a PR, and report back to the parent orchestrator task. Use inside child tasks spawned by /orchestrate."
---

# Implement Ticket

You are a child task spawned by an orchestrator. Your job is to implement one ticket, ship it as a PR, and report back.

## What you have

Your initial message contains:
- The full ticket: what to build, acceptance criteria, blocked-by list
- `SPEC: .plans/<slug>.md` — the feature spec, **in your checkout**. You are
  started on a branch that carries it. Read the file.
- `ORCHESTRATOR_TASK_ID: <id>` — the parent task to report back to
- `STACK_BASE_BRANCH: <branch>` — the branch you were started on and must add
  your ticket branch above.
- `BASE_BRANCH: <branch>` — compatibility alias for `STACK_BASE_BRANCH`.
- `STACK_BASE_PR: <pr-url-or-number>` — the PR for the branch below you; use
  it to recover local `gh stack` tracking.
- `STACK_BRANCH: <branch>` — the ticket branch to create with
  `moat gh stack add`.

The spec in your checkout is the one the orchestrator chose for you, and it is
correct **as of when you were spawned**. Do not fetch, rebase, or pull to see
whether it has changed — keeping it current is the orchestrator's job, not
yours, and a sibling's in-flight amendment is not yours to adopt. If the spec
looks wrong or contradicts your ticket, that is a blocker: say so (Step 4) and
wait.

Extract the orchestrator task ID from your message:

```bash
# It appears as a line: ORCHESTRATOR_TASK_ID: <uuid>
```

---

## Step 1 — Read and plan

Read the ticket. Understand:
- What behaviour to build (end-to-end, all layers)
- The acceptance criteria
- The spec at the `SPEC:` path — it fixes design decisions the ticket assumes.
  Read the file, not a summary of it. If the path is missing from your
  checkout, that is a blocker; do not proceed on the ticket alone.

Explore the relevant parts of the codebase before writing a line.

---

## Step 2 — Implement

Implement according to the relevant development guidelines in available skills
and the repository's `AGENTS.md` files. Run typechecking regularly, run focused
single test files regularly, and run the full relevant test suite once before
submitting the stack.

Do not implement anything outside the ticket's scope.

### Where a finding goes

Implementation turns up things the spec did not know: a hazard that rules out
the obvious approach, a decision that turns out to be wrong, a constraint nobody
saw. Report those findings to the orchestrator instead of changing the spec
yourself:

```bash
moat send --task <ORCHESTRATOR_TASK_ID> \
  --text "Finding for spec/orchestration:
Ticket: <NN> — <title>
Finding: <what changed in your understanding>
Impact: <why it matters to this ticket or later tickets>
Proposed handling: <continue with X, pause for decision, or re-scope>"
```

---

## Step 3 — Add to stack

Adding a stack layer is mandatory — every ticket ships as its own branch and PR
inside the orchestrator's feature stack.

Recover stack tracking and add your ticket branch above the branch you were
spawned on:

```bash
moat gh stack checkout <STACK_BASE_PR>
moat gh stack top
moat gh stack add <STACK_BRANCH>
```

Run those commands before coding if possible. If you already coded on a local
branch, commit or stash your work, recover the stack, create `STACK_BRANCH`,
then replay your work onto it.

Commit your work with a conventional commit message, then submit the stack:

```bash
moat gh stack submit --auto --task "$TASK_ID"
```

After `stack submit` creates or updates the PR, set the ticket PR body:

```bash
moat gh pr edit <STACK_BRANCH> --body "$(cat <<'EOF'
## Summary

<1-3 bullets of what changed and why>

## Ticket

<ticket title>

## Acceptance criteria

- [ ] Criterion 1
- [ ] Criterion 2
EOF
)"
```

Use `--open` only when the orchestrator asked for ready-for-review PRs. Note
your ticket PR URL from `moat gh stack view --json` or `moat gh pr view
<STACK_BRANCH> --json url`.

If the stack metadata is missing, stop and ask the orchestrator to respawn or
send the missing `STACK_BASE_PR`, `STACK_BASE_BRANCH`, and `STACK_BRANCH`.

Note the PR URL from the output.

---

## Step 4 — Report back

Send the result to the orchestrator:

```bash
moat send --task <ORCHESTRATOR_TASK_ID> \
  --text "PR ready: <PR URL>
Branch: <your PR branch>
Stack base: <STACK_BASE_BRANCH>
Ticket: <NN> — <title>
Findings sent: <yes/no, and short summary if yes>
All acceptance criteria implemented and tests passing."
```

**Submit the stack before you report.** A branch that exists only in your
sandbox is a branch the orchestrator cannot review, stack on, or recover. Call
out any findings you sent to the orchestrator so the next ticket's base can be
chosen with that context.

If you hit a blocker mid-implementation:

```bash
moat send --task <ORCHESTRATOR_TASK_ID> \
  --text "Blocked on: <clear description of the blocker>
Ticket: <NN> — <title>
Awaiting guidance before continuing."
```

Then wait for a reply from the orchestrator before continuing.
