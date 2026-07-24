---
name: implement-ticket
description: "Implement a single orchestrated ticket, create a PR, and report back to the parent orchestrator task. Use inside child tasks spawned by /orchestrate."
---

# Implement Ticket

You are a child task spawned by an orchestrator. Your job is to implement one ticket, ship it as a PR, and report back.

## What you have

Your initial message contains:
- The full ticket: what to build, acceptance criteria, blocked-by list
- The full feature spec, inlined — you run in your own sandbox and cannot read
  the orchestrator's spec/plan files; the message is your only copy
- `ORCHESTRATOR_TASK_ID: <id>` — the parent task to report back to
- Optionally `BASE_BRANCH: <branch>` — your ticket is stacked on an unmerged
  dependency PR; branch from it and target your PR at it (see Step 4)

Extract the orchestrator task ID from your message:

```bash
# It appears as a line: ORCHESTRATOR_TASK_ID: <uuid>
```

---

## Step 1 — Read and plan

Read the ticket. Understand:
- What behaviour to build (end-to-end, all layers)
- The acceptance criteria
- The inlined spec — it fixes design decisions the ticket assumes; if the
  message references a spec you don't have, ask the orchestrator to send it

Explore the relevant parts of the codebase before writing a line.

---

## Step 2 — Implement

Use `/dev-loop` to implement the work:
- Run type-checking after each significant change: `bun run type:check` / `cargo clippy`
- Run the relevant test file regularly
- Follow repo conventions in `.claude/rules/`

Use `/tdd` at pre-agreed seams where tests are meaningful:
- Write a failing test first
- Write the minimum code to pass it
- One seam, one cycle at a time

Do not implement anything outside the ticket's scope.

---

## Step 3 — Verify

Before creating the PR:
- Run the full test suite: `bun run test` / `cargo test -p <crate>`
- Run type-checking: `bun run type:check`
- Run lint: `bun run biome:check`

Fix any failures before proceeding.

---

## Step 4 — Create PR

Creating a PR is mandatory — every ticket ships as its own PR so the
orchestrator can stack PRs by ticket.

Work on a dedicated branch named after the ticket (e.g.
`ticket-<NN>-<slug>`). If your message contains `BASE_BRANCH: <branch>`,
start from that branch (`git fetch origin <branch> && git checkout -b
ticket-<NN>-<slug> origin/<branch>`) and pass `--base <branch>` to
`moat gh pr create`; otherwise base on the default branch.

Commit your work with a conventional commit message, then create a PR:

```bash
moat gh pr create \
  --title "<type>(<scope>): <short description>" \
  --base <BASE_BRANCH, if given> \
  --body "$(cat <<'EOF'
## Summary

<1-3 bullets of what changed and why>

## Ticket

<ticket title>

## Acceptance criteria

- [ ] Criterion 1
- [ ] Criterion 2

🤖 Implemented by Claude Code (child task of orchestrator)
EOF
)"
```

Note the PR URL from the output.

---

## Step 5 — Report back

Send the result to the orchestrator:

```bash
moat send --task <ORCHESTRATOR_TASK_ID> \
  --text "PR ready: <PR URL>
Branch: <your PR branch>
Ticket: <NN> — <title>
All acceptance criteria implemented and tests passing."
```

If you hit a blocker mid-implementation:

```bash
moat send --task <ORCHESTRATOR_TASK_ID> \
  --text "Blocked on: <clear description of the blocker>
Ticket: <NN> — <title>
Awaiting guidance before continuing."
```

Then wait for a reply from the orchestrator before continuing.
