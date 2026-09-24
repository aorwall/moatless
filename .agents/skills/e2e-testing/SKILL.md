---
name: e2e-testing
description: Run and write the Moatless Vibe E2E suite (tests/api chains). One suite, two profiles — docker and k8s — driven through the moat CLI. Use when running tests, writing new chains, setting up the test environment, sweeping leftovers, or debugging failures.
---

# E2E Testing

## Overview

There is **one** E2E suite, in `tests/`, and it runs against **two** environments:

| Profile | Environment | Auth | Fixture repository |
|---------|-------------|------|--------------------|
| `docker` | local Docker Compose at `http://localhost:8080` | password login, seeded users | cloned from a template URL |
| `k8s` | the deployed cluster, from **inside a sandbox** | the sandbox's own login (ambient) | the pre-existing Moatable repo |

Every call the suite makes goes through the compiled `moat` binary, so the tests
exercise the client people actually run — and the same chain works in both
environments without branching on which one it is in.

`E2E_PROFILE` selects the profile. Unset, a process carrying a sandbox login is
a `k8s` run and anything else is `docker`. The resolved profile is printed at the
top of every run; it is never left implicit.

For live runtime diagnosis, use [sandbox-troubleshooting](../sandbox-troubleshooting/SKILL.md).
For installation verification, use [manual smoke verification](references/manual-smoke.md).

---

## Capabilities, not tiers

A test that cannot run in an environment **skips there, with the reason named**.
It is never deleted, and it never fails for the wrong reason. What an
environment can do is declared in `tests/shared/profile.ts`:

| Capability | Means | `docker` | `k8s` |
|---|---|---|---|
| `seedUsers` | create users and log in as them | yes | no — the deployed auth provider has no local users |
| `resetDatabase` | truncate between runs | yes | no — the database is shared with real work |
| `directDatabase` | read/write Postgres directly | yes | no — no database route from a sandbox |
| `inboundCallbacks` | the backend can reach a server the test process opens | yes | no |
| `agentCredentials` | agent-tier chains can run | when a key is set | yes |
| `githubRepositories` | create and delete real repositories on GitHub | when `GITHUB_PAT` is set | when `GITHUB_PAT` is set |
| `localImages` | run an image built here and pushed nowhere | yes | no — the cluster pulls from a registry |
| `storedWorkspace` | read a stopped task's files from the store | no — the compose stack configures no bucket | yes |

In a spec:

```typescript
import { missingCapability } from '../../shared/profile'

const reason = missingCapability('seedUsers')
test.skip(reason !== null, reason ?? '')
```

The run summary groups skips by reason and prints them under **Did not run**, so
a tier that silently stopped executing cannot hide inside a skip count.

---

## Running it

### Against Docker Compose

Whole stack, built and torn down for you (this is the per-PR gate):

```bash
task test:compose:e2e                       # api suite
task test:compose:e2e -- --skip-build --keep-running
task test:compose:e2e -- --suite api -- tests/api/specs/task-crud.spec.ts   # one chain
```

`GITHUB_PAT` gives the run the `githubRepositories` capability; without it the
tests that create real repositories on GitHub skip, and the header line at the
top of the run says so. `ADMIN_PASSWORD` defaults to the CI overlay's value.

Against a stack that is already running:

```bash
bun run test:e2e                            # tests/api
bunx playwright test --config tests/api/playwright.config.ts tests/api/specs/task-crud.spec.ts
```

Always pass `--config`. Without it Playwright uses its own defaults — a 30s
timeout and no global setup — and the run fails in ways the suite did not mean.
Do not pass `--reporter` either: it replaces the config's reporters, and the run
summary is the part worth reading.

### Against the cluster

Run it **from inside a sandbox**, which is the only place with a login for the
deployed environment:

```bash
E2E_PROFILE=k8s bun run test:e2e
```

If it fails with an unknown `moat api` subcommand, the `moat` on PATH is older
than the suite's driver. Build the one in the checkout and point at it:

```bash
cargo build -p moatless-cli --bin moat
MOAT_BIN="$PWD/target/debug/moat" E2E_PROFILE=k8s bun run test:e2e
```

A cluster run takes about ten minutes at the default three workers; most of that
is real pods scheduling and pulling. Nightly, `.moatless/loops/e2e-nightly/LOOP.md`
does exactly this on a schedule.

---

## Run isolation and sweeping

The cluster database is shared and long-lived — there is no truncate to fall
back on. So every resource a test creates is named through `scopedName()` with
the run's own prefix (`tests/shared/run-scope.ts`):

```
e2e-mfz3k1p-a4f9-Task Filter Repo
└─┬─┘└──┬──┘└─┬─┘
  │     │     └── random, so two runs starting the same millisecond differ
  │     └──────── start time, base 36
  └────────────── the family every run shares
```

The prefix carries the start time, so anything can decide what is stale from the
name alone. Teardown sweeps the run's own prefix. For runs that were killed
before teardown — an open task keeps a sandbox pod alive — sweep by hand:

```bash
E2E_PROFILE=k8s bun tests/sweep.ts --dry-run       # say what would go
E2E_PROFILE=k8s bun tests/sweep.ts --older-than 3h # default is 3h
```

Only resources whose name says a run created them are ever touched.

---

## Layout

```
tests/
  shared/
    profile.ts        # the two profiles: URLs, auth, fixtures, timeouts, capabilities
    moat.ts           # the driver — spawns the `moat` binary; json/text/raw/stream
    moat-api.ts       # domain verbs on top of the driver
    run-scope.ts      # the run prefix, scopedName / scopedSlug
    sweep.ts          # removing what a run left behind
    event-stream.ts   # SSE collected through the CLI
  api/
    playwright.config.ts
    global-setup.ts / global-teardown.ts
    specs/*.spec.ts   # the tests — a serial chain, or standalone cases
    steps/            # actions that compose (createTask, waitForSandboxReady, …)
    helpers/          # progress(), discardTask(), the GitHub and DB helpers
    fixtures/         # `api`, `events`, and the auth fixtures (alice/bob)
    context/          # ChainContext, chainEvents() — what one test hands the next
    reporters/        # the run summary
  sweep.ts            # the standalone sweeper
```

A chain is a `test.describe` with `test.describe.configure({ mode: "serial" })`:
each test is one step, and a step that fails stops the chain rather than
cascading misleading failures. Separate chains run in parallel. About half the
files in `specs/` are chains; the rest are standalone cases that set up and tear
down their own state, which is why the directory is not called `chains/`.

---

## Writing a chain

1. **Find the existing coverage.** Read nearby chains in `tests/api/specs/` and
   the domain helpers they use. Scenario ids live in test titles and are never
   reused.
2. **Put it in the chain that owns the domain** —
   `tests/api/specs/<domain>-*.spec.ts`.
3. **Speak through the fixtures**, not through raw HTTP — `api` for a single
   call, a step from `../steps` when the action composes:

Use [task-crud.spec.ts](../../../tests/api/specs/task-crud.spec.ts) as a complete
fixture and cleanup example. Initialize the repository and ChainContext before
calling a creation step; do not rely on an undeclared `ctx`.

Rules that keep the suite worth running:

- **Every assertion carries a message saying what is wrong**, not what was
  compared. The run summary prints that sentence as the failure; `expect(a).toBe(b)`
  reports nothing anyone can act on.
- **An assertion that cannot fail is worse than none.** Asserting a value equals
  itself, or that a list "has entries" when a fresh environment has them anyway,
  reads as coverage and is not.
- **Timeouts come from the profile**, never from a constant — a pod takes
  several times as long as a local container. A test states which transitions it
  waits on and lets `budget("sandboxReady", "agentIdle")` do the arithmetic; the
  waits themselves already default to the profile's ceiling.
- **Name every resource through `scopedName`/`scopedSlug`**, or the sweep cannot
  clean it up.
- **Guard on a capability, don't branch on the profile name.**
- **Call `api` directly for a single request.** A step exists to compose — a
  call plus the wait that makes it meaningful, a default, a projection. A
  wrapper that only reorders arguments hides which endpoint ran.
- **Narrate with `progress()`**, which labels the line with the test that
  emitted it. Three chains interleave their output.
- **A `// ─── ID: title ───` banner indexes a scenario the file-top doc lists.**
  Any other comment that restates the test title or the assertion below it is
  a cut. The comment rules in the root `AGENTS.md` apply.

---

## Debugging

- The run summary at the end names the profile, the run prefix, the counts, each
  failure with its assertion, and every skip with its reason.
- HTML report: `test-results/api/` (`npx playwright show-report test-results/api`).
  JSON: `test-results/api/results.json`.
- `[global-setup]` lines at the top say which environment was resolved and which
  fixture repository was found; the header line names every capability the
  profile lacks, which is why a group of tests will skip.
- Docker stack logs: `docker compose -f docker-compose/docker-compose.yml -f docker-compose/docker-compose.ci.yml logs backend`.
- Cluster: `moat tasks logs <task-id>`, and `moat tasks diagnose <task-id>`.

**Common causes**

| Symptom | Cause |
|---|---|
| "Backend is not healthy" on every chain | wrong `E2E_PROFILE`, or a `moat` older than the suite's driver — set `MOAT_BIN` |
| "Fixture repository … is missing" | `E2E_REPOSITORY_ID` points at something that is not there |
| a whole tier skipped | the profile lacks the capability — read the **Did not run** group |
| a chain times out at 30s | `--config` was not passed |
| leftovers on the cluster | a run was killed before teardown — `bun tests/sweep.ts` |
