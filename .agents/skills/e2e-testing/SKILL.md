---
name: e2e-testing
description: Run and write the Moatless Vibe E2E suite (tests/api chains) through scripts/local-e2e/run.sh — a backend built from the checkout, its own Postgres schema, and every task's sandbox a local process. Use when running tests, writing new chains, running the CI E2E job, sweeping leftovers, or debugging failures.
---

# E2E Testing

## Overview

The E2E suite lives in `tests/`. `scripts/local-e2e/run.sh` runs it against a
throwaway stack that needs no Docker image and no cluster:

1. It creates a schema named `e2e_<epoch>_<hex>` on an existing Postgres and
   selects it with `search_path` in `DATABASE_URL`. The backend migrates it on
   startup; `crates/core/src/db/migrations.rs` scopes the migrations' catalog
   probes to that schema.
2. It starts `moatless-backend` from the checkout on free ports, with
   `SANDBOX_PROVIDER=local`. Each task's sandbox is then a `sandbox` child
   process under `/tmp/le-<run>/tasks/<task-id>/`, running the real
   `claude` and `codex` CLIs.
3. When `OPENAI_API_KEY` is set, it seeds a global Codex connection.
4. It runs Playwright under the `local` profile, then drops the schema and
   removes the run's directories.

Every call the suite makes goes through the compiled `moat` binary, so the tests
exercise the client people actually run.

For live runtime diagnosis, use [sandbox-troubleshooting](../sandbox-troubleshooting/SKILL.md).

---

## Running it

Run it through `moat cmd run`: a build plus the suite outlasts a shell call.

```bash
scripts/local-e2e/run.sh                          # build, then the whole api suite
scripts/local-e2e/run.sh --skip-build -- tests/api/specs/agent-codex-flow.spec.ts
scripts/local-e2e/run.sh --skip-build --keep -- <specs>   # leave schema and logs
MODEL_GATEWAY_HUB_URL=<hub> scripts/local-e2e/run.sh --standalone -- <specs>   # no cluster
scripts/local-e2e/run.sh sweep --dry-run          # what killed runs left behind
scripts/local-e2e/run.sh sweep --older-than-hours 0
```

`--standalone` is for a sandbox with no cluster credentials, where
`sync-env.sh` cannot run. It starts a throwaway Postgres in the run directory,
fetching the `@embedded-postgres` npm build when no server is installed, and
writes the CI job's base env there. It needs `MODEL_GATEWAY_HUB_URL` and
`MODEL_GATEWAY_HUB_KEY`, and puts the hub on both providers' keys and base URLs
in place of any key or login the caller holds. With `--keep`, the hub key
stays in the run directory's `backend.env` (mode 0600) until you remove it.

Everything after `--` goes to Playwright. The harness passes
`--config tests/api/playwright.config.ts` itself; do not pass `--reporter`,
which replaces the run summary.

**Inputs**

| Variable | Effect |
|---|---|
| `LOCAL_E2E_BASE_ENV` | Env file the backend starts from, and the source of `DATABASE_URL`. Defaults to `/run/moatless/shadow/backend.env`, which `scripts/shadow/sync-env.sh` writes from the deployment when it is missing. |
| `LOCAL_E2E_BIN_DIR` | Where `moatless-backend`, `sandbox` and `moat` are. Defaults to `target/debug`. |
| `ANTHROPIC_API_KEY` / `CLAUDE_CODE_OAUTH_TOKEN` | Passed to each sandbox; enables the Claude chains. |
| `OPENAI_API_KEY` | Seeds the Codex connection; enables the Codex chains. |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` | Stored as Secrets beside the keys. The gateway calls them only when they are the base env's `MODEL_GATEWAY_HUB_URL` and its `/v1`. |
| `GITHUB_PAT` | Enables the chains that create real GitHub repositories. |
| `E2E_WORKERS` | Playwright workers. |

Build with `CARGO_BUILD_JOBS=4`: an unbounded build reaches the sandbox's
memory cap.

**Do not run `pkill -f` against the backend or the sandboxes.** The pattern also
matches this sandbox's own processes. The harness stops a run's processes on
exit by finding its sandbox root in their environment, which the backend, every
sandbox and every agent carry.

### In CI

`.github/workflows/e2e.yml` runs the harness on manual dispatch (Actions → E2E →
Run workflow). `build-binaries` compiles the three binaries with
`--profile ci`. The `playwright` job then:

- runs in `moatless-sandbox-base:latest`, which ships the agent CLIs;
- starts `postgres:16` as a service container;
- writes a base env: `DATABASE_URL`, generated `SECRETS_ENCRYPTION_KEY` and
  `TASK_TOKEN_SECRET`, and the CLIProxyAPI hub as `MODEL_GATEWAY_HUB_URL` and
  `MODEL_GATEWAY_HUB_KEY`;
- passes the hub key as both `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`, with
  base URLs naming the hub, so the run holds no provider key;
- runs `run.sh --skip-build --keep`;
- uploads `test-results/`, the backend log and each sandbox's log.

A change to what the harness needs from its environment has to land in that job
too.

The `run_trajectory_cli` input runs the real-CLI trajectory tier
(`crates/sandbox/tests/trajectory_e2e.rs`) instead: `build-binaries` compiles
its test binary, and `trajectory-cli` runs it in the same base image with both
agents' credentials. Locally:
`TRAJECTORY_E2E=1 cargo test -p moatless-sandbox --test trajectory_e2e -- --ignored`.

---

## Capabilities, not tiers

A test that cannot run in an environment **skips there, with the reason named**.
It is never deleted, and it never fails for the wrong reason.
`tests/shared/profile.ts` declares what the `local` profile can do:

| Capability | Means | `local` |
|---|---|---|
| `seedUsers` | create users and log in as them | yes |
| `resetDatabase` | truncate between runs | no — each run gets a new schema |
| `directDatabase` | read/write Postgres directly | no — the helpers use `docker exec` |
| `inboundCallbacks` | the backend can reach a server the test process opens | yes |
| `claudeCredentials` | Claude agent chains can run | when `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` is set |
| `codexConnection` | Codex agent chains can run | when `OPENAI_API_KEY` is set |
| `githubRepositories` | create and delete real repositories on GitHub | when `GITHUB_PAT` is set |
| `localImages` | run an image built here | no — sandboxes are processes |
| `storedWorkspace` | read a stopped task's files from the store | no — no bucket |
| `artifactPages` | publish an Artifact and open its page with a token | when the base env sets `S3_BUCKET_NAME` and `SESSION_JWT_SECRET` or `TASK_TOKEN_SECRET` |
| `sandboxReadyEvent` | a watcher publishes `sandbox.ready` | no — `sandbox-local` has no lifecycle watcher |
| `workspaceImage` | the runtime runs the workspace's `dockerImage` | no — sandboxes run on the host |

In a spec:

```typescript
import { missingCapability } from '../../shared/profile'

const reason = missingCapability('seedUsers')
test.skip(reason !== null, reason ?? '')
```

The run summary groups skips by reason and prints them under **Did not run**, so
a tier that silently stopped executing cannot hide inside a skip count.

---

## Run isolation and sweeping

A run owns three things, each named after its schema, so the sweeper can tell a
stale one from its name alone:

| What | Where |
|---|---|
| the schema | `e2e_<epoch>_<hex>` on the base env's database |
| the sandboxes' files | `/tmp/le-<epoch>_<hex>/` (short, so the credential socket path under it fits 108 bytes) |
| the backend log and Playwright's `HOME` | `/run/moatless/local-e2e/e2e_<epoch>_<hex>/` |

The harness removes all three on exit unless `--keep` is passed. A killed run
leaves them behind, with its processes still running; `run.sh sweep` stops
those processes and removes whatever is older than three hours by default. It
touches only names matching that pattern.

Inside a run, name every resource a test creates through
`scopedName()`/`scopedSlug()` (`tests/shared/run-scope.ts`). Teardown sweeps
the run's own prefix, so chains running in parallel never collide.

---

## Layout

```
scripts/local-e2e/run.sh   # the harness and its sweeper
crates/sandbox-local/      # SANDBOX_PROVIDER=local: each sandbox a child process
tests/
  shared/
    profile.ts        # URLs, auth, fixtures, timeouts, capabilities
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
   call, a step from `../steps` when the action composes.

Use [task-crud.spec.ts](../../../tests/api/specs/task-crud.spec.ts) as a complete
fixture and cleanup example, and
[agent-codex-flow.spec.ts](../../../tests/api/specs/agent-codex-flow.spec.ts) for a
short agent chain. Initialize the repository and ChainContext before calling a
creation step; do not rely on an undeclared `ctx`.

Rules that keep the suite worth running:

- **Every assertion carries a message saying what is wrong**, not what was
  compared. The run summary prints that sentence as the failure; `expect(a).toBe(b)`
  reports nothing anyone can act on.
- **An assertion that cannot fail is worse than none.** Asserting a value equals
  itself, or that a list "has entries" when a fresh environment has them anyway,
  reads as coverage and is not.
- **Timeouts come from the profile**, never from a constant. A test states which
  transitions it waits on and lets `budget("sandboxReady", "agentIdle")` do the
  arithmetic; the waits themselves already default to the profile's ceiling.
- **Name every resource through `scopedName`/`scopedSlug`**, or teardown cannot
  clean it up.
- **Guard on a capability, don't branch on the profile name.**
- **Call `api` directly for a single request.** A step exists to compose — a
  call plus the wait that makes it meaningful, a default, a projection. A
  wrapper that only reorders arguments hides which endpoint ran.
- **Narrate with `progress()`**, which labels the line with the test that
  emitted it. Parallel chains interleave their output.
- **A `// ─── ID: title ───` banner indexes a scenario the file-top doc lists.**
  Any other comment that restates the test title or the assertion below it is
  a cut. The comment rules in the root `AGENTS.md` apply.

---

## Debugging

Pass `--keep` to inspect a failed run; the harness prints what it kept.

- **Run summary:** the end of the output names the profile, the run prefix, the
  counts, each failure with its assertion, and every skip with its reason.
  `test-results/api/results.json` holds the same as JSON.
- **Backend:** `/run/moatless/local-e2e/<schema>/backend.log`.
- **A sandbox:** `/tmp/le-<run>/tasks/<task-id>/sandbox.log`, beside that
  task's `workspace/`, `home/` and `tmp/`.
- **Data:** connect with `DATABASE_URL` from the base env plus
  `PGOPTIONS="-c search_path=<schema>"`.

**Common causes**

| Symptom | Cause |
|---|---|
| "Backend not healthy after 180s" or "exited before it was healthy" | the harness prints the log's tail; a migration error there usually means an unscoped catalog probe (`migrations.rs`) |
| a whole tier skipped | the profile lacks the capability — read the **Did not run** group |
| an agent turn never completes | `EVENT_HANDLER_ENABLED` is off in the backend's env |
| psql or the backend lands in the wrong schema | the base env's `DATABASE_URL` already carries `options=`; the harness strips it, so a hand-built URL must too |
