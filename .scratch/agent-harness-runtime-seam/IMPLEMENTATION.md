# Agent harness runtime seam — implementation guide

Work from `agent/agent-harness-runtime-seam`. Implement one ticket at a time
from `issues/`; do not pull later-ticket scope into the current change.

## Read before editing

Read these sources in order:

1. `CONTEXT.md` for the canonical domain language.
2. `docs/adr/0004-agent-harness-is-fixed-per-task.md`.
3. `docs/adr/0005-agent-harness-modules-return-outcomes.md`.
4. ADR-0002 and ADR-0003 before changing source events or Message assembly.
5. `specs/agent-harness-runtime-seam.md` in full.
6. The assigned ticket, including every blocker and acceptance criterion.
7. The path-scoped repository rules for every production or test file changed.

When existing code conflicts with an accepted decision, follow the ADRs and
specification. Record a genuinely new decision in an ADR rather than weakening
the seam locally.

## Ticket order

Tickets 01, 02, and 03 can begin independently. The remaining order is:

```text
01  02  03
         |
         04
       /    \
      05     07
      |
      06
       \    /
         08
          |
         09
          |
         10
```

## Invariants to preserve

- A Task selects one immutable Agent harness.
- A Sandbox owns one in-memory instance of that harness.
- At most one Agent CLI process is active in a Sandbox.
- A Session can survive Agent process replacement and Sandbox recreation.
- The coordinator owns Run identity, Turn state, acknowledgement, retry policy,
  credential policy, and backend publication.
- The harness owns Agent process lifecycle, Session start/resume, input
  delivery, cancellation, native normalization, and error classification.
- The external harness interface stays limited to submitting input, cancelling
  active execution, and receiving the next signal.
- Every started execution produces exactly one terminal outcome.
- Pending Messages are acknowledged only after `InputAccepted`.
- One canonical source event represents one native content block.
- Message assembly happens only in the backend.
- Resume failure never silently creates a fresh Session.
- Existing public HTTP and Message shapes remain compatible during migration.

Use Agent harness, Agent, Session, Run, Turn, and Message exactly as defined in
`CONTEXT.md`. Do not reintroduce “Agent runtime” or “Agent type.”

## Module shape

- Build a deep harness module with a small interface.
- Keep implementation selection private; do not add a public universal trait.
- Split protocol, ordering, Session rules, process lifecycle, normalization,
  native identity, and error facts by cohesive responsibility.
- Keep native normalizers pure where practical.
- Keep module roots focused on re-exporting the interface.
- Treat roughly 200–300 production lines as a review signal. Do not create
  pass-through wrappers just to meet a line target.
- Do not add a trait with one production implementation.
- Preserve one explicit migration compatibility path, then remove it in ticket
  10.

## Per-ticket development loop

1. Confirm every blocker is already implemented.
2. Locate the current behavior and its tests before editing.
3. Define the missing observable test or failing regression first.
4. Implement only the assigned ticket.
5. Add or update Rust integration coverage in `crates/integration-tests/` for
   behavior that crosses the backend, Sandbox, database, or Agent process.
6. Run formatter, clippy with warnings denied, and full tests for every changed
   Rust crate.
7. Run focused integration tests when they are useful for the ticket's feedback
   loop; do not run the complete E2E matrix after every intermediate ticket.
8. Report exact commands and results.

Use scripted CLI adapters for deterministic lifecycle contracts. Add E2E cases
alongside the ticket that introduces their behavior, but defer the complete E2E
verification run until ticket 10 has integrated the whole seam.

## Final integrated E2E verification

Read and follow the `e2e-testing` skill before writing or running E2E tests.
Ticket 10 owns the complete Docker Compose regression gate; earlier tickets do
not each rerun this matrix.

```bash
# Database-backed API and Message behavior
task test:compose:db

# Sandbox lifecycle and cross-process behavior
task test:compose:sandbox

# Real Agent execution; requires the relevant CLI credentials
CLAUDE_CODE_OAUTH_TOKEN=... task test:compose -- --tier agent

# Focused iteration
task test:compose -- --test <test-file> --keep-running

# Full Rust integration regression gate
CLAUDE_CODE_OAUTH_TOKEN=... task test:compose:all

# Public API chains against Docker Compose
task test:compose:e2e
```

Tests in the Sandbox and Agent tiers must use their documented environment
gate and self-skip when it is absent. Do not silently omit a gated test: state
which command was not run, which credential or executable was missing, and
what lower-level coverage did run.

Across the suite, prove externally observable ordering, acknowledgement,
source-block preservation, exactly-one terminal outcome, process-tree
cancellation, Claude injection into the current Run, Codex queueing into the
next Run, Session restoration, and equivalence of persisted reads and live
Message upserts.
