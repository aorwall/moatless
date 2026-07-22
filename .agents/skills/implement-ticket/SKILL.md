---
name: implement-ticket
description: "Implement a single orchestrated ticket, create a PR, and report back to the parent orchestrator task. Use inside child tasks spawned by /orchestrate."
---

# Implement Ticket

You are a child task spawned by an orchestrator. Your job is to implement one ticket, ship it as a PR, and report back.

## What you have

Your initial message contains:
- The full ticket: what to build, acceptance criteria, blocked-by list
- `ORCHESTRATOR_TASK_ID: <id>` — the parent task to report back to

Extract the orchestrator task ID from your message:

```bash
# It appears as a line: ORCHESTRATOR_TASK_ID: <uuid>
```

---

## Step 1 — Read and plan

Read the ticket carefully. Understand:
- What behaviour to build (end-to-end, all layers)
- The acceptance criteria
- Any mentions of the spec for additional context

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

Commit your work with a conventional commit message, then create a PR:

```bash
moat gh pr create \
  --title "<type>(<scope>): <short description>" \
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
