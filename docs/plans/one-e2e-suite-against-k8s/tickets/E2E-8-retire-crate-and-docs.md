# E2E-8 — Retire the Rust crate and correct the documentation

- **Priority:** P2
- **Status:** Done
- **Blocked by:** E2E-5 (coverage ported), E2E-6 (cluster profile green)
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

Delete what the rewrite replaced, and fix the documents that describe it.

**Delete** `crates/integration-tests/` and its workspace membership. Nothing may
be deleted before E2E-5 records that its assertions live in the TypeScript suite.

**Update** every place that tells someone how to run E2E tests:

| Surface | What is wrong after the rewrite |
|---|---|
| `.github/workflows/e2e.yml` | offers a Rust tier that no longer exists |
| `Taskfile.yml` | `test:compose*` targets point at the deleted crate |
| `CLAUDE.md` | the Integration & E2E command block |
| `.claude/skills/e2e-testing/` | describes two suites, tiers by env gate, and a Docker-only world |
| `docker-compose/test-runner.sh` | its Rust path; the Playwright path stays |

**Correct or remove two stale documents.** `docs/e2e-testing-strategy.md` and
`docs/kubernetes-integration-tests.md` describe a codebase that no longer exists
— they reference a TypeScript backend under `apps/backend/` and test paths that
were removed. They currently mislead anyone searching for how to test against a
cluster, which is exactly the question this plan answers. Rewrite them around the
profile model, or delete them and point at the spec.

## Acceptance criteria

- [x] The Rust integration-tests crate is gone, and the workspace builds.
- [x] No document or script references the deleted crate.
- [x] The e2e-testing skill describes one suite and two profiles.
- [x] The two stale documents are corrected or removed.
- [x] A newcomer can find how to run against the cluster from `CLAUDE.md` alone.

## Verification

- `cargo build` and `cargo clippy -- -D warnings` across the workspace.
- A grep for the crate name across the repository returns nothing.
- Follow the instructions in the updated skill from scratch, under both
  profiles, and confirm each command exists and works.

## Closing notes

### Deleted

| Path | Why |
|---|---|
| `crates/integration-tests/` (33 files) | Its assertions live in `tests/api/` — the reconciliation is E2E-5's closing notes, file by file |
| `docker-compose/test-runner.sh` | Every path in it was `cargo test -p moatless-integration-tests`. The Playwright runner is a separate, self-contained script |
| `docs/e2e-testing-strategy.md` | 490 lines about a Vitest/`apps/backend` world: a test pyramid with "Integration Tests (Vitest)", `tests/e2e/docker/specs/`, `playwright.docker.config` — none of which exist |
| `docs/kubernetes-integration-tests.md` | Described running `kubernetes-environment-driver.integration.test.ts` as a k8s Job from `apps/backend/src/features/...` — a TypeScript backend the repo has not had for a long time |
| `docker/build-test-runner.sh` | The image that document told you to rebuild. It rsyncs `apps/backend/` into a bun image; it has been unbuildable for as long as that directory has been gone |

The two documents were removed rather than rewritten. Nothing linked them, and
what a reader needs instead — one suite, two profiles, how to run against the
cluster — is now in the `e2e-testing` skill, with `CLAUDE.md` carrying the four
commands.

### Updated

| Surface | Change |
|---|---|
| `Cargo.toml` | workspace member dropped; `Cargo.lock` regenerated |
| `.github/workflows/e2e.yml` | the `integration_tier` input and its job are gone. The `sandbox-replacement` job went with them: it invoked the deleted runner for `sandbox_replacement_ordering`, which is now a chain of the api suite, so its Codex credentials moved onto the Playwright job that runs it |
| `Taskfile.yml` | `test:compose`, `:db`, `:sandbox`, `:all` removed; `test:compose:e2e` is the Docker-profile entry point |
| `CLAUDE.md` | the tier block replaced by the profile block, including the cluster command and the sweep |
| `.agents/skills/e2e-testing/SKILL.md` | rewritten: profiles and capabilities instead of tiers and env gates, run isolation and sweeping, how to write a chain, and a symptom→cause table for the failures this plan actually hit |
| `.agents/skills/testing-guidelines/SKILL.md`, `.agents/skills/dev-loop/SKILL.md` | one-line pointers corrected |
| `.moatless/loops/code-review/LOOP.md` | the reviewer's file-classification table no longer routes `crates/integration-tests/**` |
| `docker-compose/playwright-runner.sh` | header no longer calls itself a companion to a script that is gone |
| `docs/docker-builds.md`, `docs/fork-maintenance.md`, `docs/plans/backend-crate-split.md`, `specs/plugins-terminology-and-scope.md` | commands and prior-art references repointed |

`specs/one-e2e-suite-against-k8s.md` still names the crate throughout, and
should: it is the record of the decision to retire it, and describing the
before-state is its job.

### Verification

- `cargo clippy --workspace -- -D warnings` — clean. `cargo build --workspace` —
  clean.
- `grep -rn "integration-tests\|test-runner.sh\|moatless-integration-tests"`
  across the repository returns only `specs/one-e2e-suite-against-k8s.md` and
  this plan's own tickets.
- The `k8s` half of the skill was followed from scratch: `E2E_PROFILE=k8s bun run
  test:e2e` (150 passed, 0 failed, 67 skipped) and `E2E_PROFILE=k8s bun
  tests/sweep.ts --dry-run` (17 connections from earlier runs, nothing else
  outstanding).
- The `docker` half could not be run: this sandbox has a `docker` binary but not
  a compose stack to bring up, and `task` is not installed in it. Each command
  was checked to exist and to be spelled the way the file it points at expects —
  the `test:compose:e2e` target, `playwright-runner.sh --suite`, and the
  spec-filter form (`-- --suite api -- tests/api/chains/task-crud.spec.ts`),
  which `docker/e2e-entrypoint.sh` forwards to `playwright test`.

### One thing worth knowing

Deleting the crate removed the only caller of `docker-compose/test-runner.sh`,
and with it the database reset it performed before every run (`TRUNCATE …
CASCADE`). The Playwright runner does not do that from the shell — the `docker`
profile resets through `tests/api/helpers/db-reset.ts` in global setup, gated on
the `resetDatabase` capability. Same effect, decided by the profile rather than
by a bash script, which is the point of the rewrite.
