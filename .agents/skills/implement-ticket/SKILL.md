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
- `BASE_BRANCH: <branch>` — the branch you were started on. Branch from it and
  target your PR at it (see Step 4). It may be the feature branch or an unmerged
  dependency's PR branch; either way it is your base.

The spec in your checkout is the one the orchestrator chose for you, and it is
correct **as of when you were spawned**. Do not fetch, rebase, or pull to see
whether it has changed — keeping it current is the orchestrator's job, not
yours, and a sibling's in-flight amendment is not yours to adopt. If the spec
looks wrong or contradicts your ticket, that is a blocker: say so (Step 5) and
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

Use `/dev-loop` to implement the work:
- Run type-checking after each significant change: `bun run type:check` / `cargo clippy`
- Run the relevant test file regularly
- Use `rust-dev-guidelines` for Rust changes and
  `frontend-dev-guidelines` for frontend changes

Use `/tdd` at pre-agreed seams where tests are meaningful:
- Write a failing test first
- Write the minimum code to pass it
- One seam, one cycle at a time

Do not implement anything outside the ticket's scope.

### Where a finding goes

Implementation turns up things the spec did not know: a hazard that rules out
the obvious approach, a decision that turns out to be wrong, a constraint nobody
saw. Those belong in the spec, not in a comment beside the code.

Amend `.plans/<slug>.md` on **your own branch**, in the same commit as the code
it explains, and name the amendment in your report (Step 5) so the orchestrator
can carry it into the next ticket's base. Never push to the feature branch or
any branch that is not yours.

Comments are for the reader of *this function*, and they follow the density of
the code around them — check a neighbouring file before deciding what is normal
here. An invariant about a function is stated once, on that function; if you
find yourself writing the same paragraph at a second call site, it belonged on
the callee. Design rationale that the spec already carries gets a pointer, not
a restatement.

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
`ticket-<NN>-<slug>`), cut from the `BASE_BRANCH` you were started on
(`git checkout -b ticket-<NN>-<slug>`), and pass `--base <BASE_BRANCH>` to
`moat gh pr create`. Never target the default branch — your base carries the
spec, and the orchestrator merges the stack down in dependency order.

Commit your work with a conventional commit message, then create a PR:

```bash
moat gh pr create \
  --title "<type>(<scope>): <short description>" \
  --base <BASE_BRANCH> \
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
Spec amended: <what you changed in .plans/<slug>.md, or 'no'>
All acceptance criteria implemented and tests passing."
```

**Push before you report.** A branch that exists only in your sandbox is a
branch the orchestrator cannot review, stack on, or recover. Call the spec
amendment out explicitly — it is the orchestrator's only signal that the next
ticket's base needs updating.

If you hit a blocker mid-implementation:

```bash
moat send --task <ORCHESTRATOR_TASK_ID> \
  --text "Blocked on: <clear description of the blocker>
Ticket: <NN> — <title>
Awaiting guidance before continuing."
```

Then wait for a reply from the orchestrator before continuing.
