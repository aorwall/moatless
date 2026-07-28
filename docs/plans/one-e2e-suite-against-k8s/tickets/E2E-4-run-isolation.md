# E2E-4 — Run isolation without truncating the database

- **Priority:** P1
- **Status:** Done
- **Blocked by:** E2E-1 (profile carries `resetDatabase`)
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

Everything needed for a run to coexist with real data, so the cluster profile has
somewhere to land. There is no tenancy boundary to lean on — resource scope is
only global or per-user, and teams grant no access (ADR 0006) — so the suite
enforces the convention on itself.

Four parts:

1. **A run prefix.** One identifier minted per run, applied to the name of every
   resource a test creates, and printed in the report header.
2. **Filtered assertions.** No assertion reads an unfiltered list. Every list is
   narrowed to the run's own prefix before anything is counted or compared. This
   is the bulk of the work — the existing tests do assert on totals, and each of
   those assertions has to change.
3. **Teardown by prefix.** Whatever the outcome, a run deletes what it created,
   including stopping any sandbox it started.
4. **A sweep mode.** A command that removes resources older than a few hours
   carrying the run prefix, with their sandboxes, and reports what it removed.
   This is what stops a crashed run from leaving sandboxes alive on a shared
   environment.

Under the `docker` profile the truncate stays available and keeps running — it is
faster, and it is the control that proves the filtered assertions are correct
rather than accidentally passing.

## Acceptance criteria

- [x] Every created resource carries the run prefix in its name.
- [x] No assertion in the suite reads an unfiltered list.
- [x] Teardown removes the run's resources after both a pass and a failure.
- [x] Sweep removes only prefixed resources past the age threshold, and says what
      it removed.
- [x] A dry-run sweep reports without deleting.
- [ ] The suite passes with the truncate disabled, against a database holding
      unrelated data.
      *(Verified for the chains the cluster can run today — task CRUD, sandbox
      errors, forward-auth. The whole suite against the cluster is E2E-6.)*

## Verification

- Against Compose with truncate disabled and pre-seeded unrelated tasks and
  repositories: the full suite passes.
- Kill a run mid-chain, then sweep: the orphaned task and its sandbox are gone,
  and the unrelated data is untouched.
- Run the suite twice without cleaning in between: the second run passes.
