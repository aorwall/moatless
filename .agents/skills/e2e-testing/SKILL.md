---
name: e2e-testing
description: Run and write E2E tests for Moatless Vibe. Covers Rust integration tests (crates/integration-tests/ — API/CLI, tiered by db/sandbox/agent), Playwright api/ui tests (tests/ — browser + API against Docker Compose), and the env-agnostic cluster suite (tests/cluster/ — runs against a local or remote k8s deployment via MOATLESS_BASE_URL + API_KEY). Use when running tests, writing new tests, running e2e against a cluster, setting up the test environment, or debugging test failures.
---

# E2E Testing

## Overview

Three suites cover E2E testing, split by **target**:

| Suite | Location | Runner | Target | What it tests |
|-------|----------|--------|--------|---------------|
| **Rust integration tests** | `crates/integration-tests/` | `cargo test` | Docker Compose | Backend API via HTTP (MoatClient/reqwest) |
| **Playwright api/ui tests** | `tests/api/`, `tests/ui/` | `bun run test:e2e` | Docker Compose | API chains + browser UI workflows |
| **Playwright cluster tests** | `tests/cluster/` | `bun run test:e2e:cluster` | local-k8s **or** remote | Live deployment as a black-box HTTP client |

The first two run against the backend at `http://localhost:8080` (Docker Compose,
the per-PR gate). The **cluster** suite runs against a live k8s deployment — see
[Cluster target](#cluster-target-env-agnostic-suite).

| Target | Auth |
|--------|------|
| Docker Compose | admin cookie / seeded users |
| local-k8s / remote | `test-api-user` Bearer key |

---

## Rust Integration Tests

### Test Tiers

Tests are organized into three tiers with environment variable gates:

| Tier | Env Gate | `cargo test` flags | Tests |
|------|----------|-------------------|-------|
| **db** | None | `-- --nocapture` | task_crud, task_errors, repo_crud, messages, channel_config, channel_errors, channel_routing, channel_cross_channel, channel_webhook_callback, auth_verify, auth_teams, auth_access |
| **sandbox** | `SANDBOX_E2E=1` | `-- --ignored --nocapture` | sandbox_lifecycle, sandbox_errors, file_operations |
| **agent** | `CLAUDE_SDK_E2E=1` | `-- --ignored --nocapture` | agent_execution_flow, agent_plan_mode, agent_stop_recovery, message_acknowledgment, plugin_repos_e2e |

The sandbox and agent tier tests use `#[ignore]` and only run with `-- --ignored`.
Single-test mode (`--test <name>`) passes `--include-ignored` automatically — every gated test self-skips when its env gate is missing.

**Special gated tests** (also `#[ignore]` + self-skip):

| Test | Env Gate | Extra backend env | What it covers |
|------|----------|-------------------|----------------|
| `github_proxy` | `GITHUB_PROXY_E2E=1` | `GITHUB_API_BASE_URL=http://host.docker.internal:18092`, `GITHUB_CHANNEL_TOKEN=<any>` | `moat gh` typed proxy: PR create + comments against an in-process mock GitHub (`src/mock_github.rs`) — verifies task binding, `Moatless task:` footer, default-branch fallback |
| `webhook_callback` | `CHANNEL_CALLBACK_E2E=1` | `CALLBACK_HOST=host.docker.internal` (test-side) | Outbound webhook callbacks to an in-process server |

```bash
# moat gh proxy E2E (mock GitHub on host port 18092):
GITHUB_PROXY_E2E=1 \
GITHUB_API_BASE_URL=http://host.docker.internal:18092 \
GITHUB_CHANNEL_TOKEN=mock-token \
  docker-compose/test-runner.sh --test github_proxy
```

### Shared Utilities (`crates/integration-tests/src/`)

**`lib.rs`** — Core helpers:
- `test_client()` — Unauthenticated `MoatClient` (reads `MOATLESS_API_URL`, defaults to `http://localhost:8080`)
- `client_as_user(login, role)` — Client authenticated via password-login session cookie (server assigns the id, returned on `TestUser.id`)
- `client_as_user_with_teams(login, role, team_ids)` — Same, with team membership
- `seed_test_users()` — Seeds alice, bob, carol via password login
- `ensure_template_repo(client)` — Creates/finds template repo (`hello-world-ui.git`)
- `reset_database()` — Truncates all application tables via `docker exec moatless-postgres psql`

**`sandbox_helpers.rs`** — Polling helpers (2s intervals):
- `wait_for_sandbox_ready(client, task_id, timeout)` — Polls until `environment_status == "ready"` with stability re-check
- `wait_for_agent_idle(client, task_id, timeout)` — Polls until `agent_state == "idle"`
- `wait_for_files_available(client, task_id, timeout)` — Polls until workspace has files
- `wait_for_sandbox_error(client, task_id, timeout)` — Polls for error state

**`callback_server.rs`** — In-process HTTP server for webhook callback testing:
- `CallbackServer::start()` — Binds ephemeral port, captures POST bodies as JSON
- `wait_for_callback(timeout)` — Waits for a webhook to arrive

**`raw_api.rs`** — Raw reqwest helpers for endpoints not in the generated API client (error cases, webhook sending, auth verify with custom headers).

### Running Rust Integration Tests

**Automated (recommended)** — builds images, starts compose, runs tests, tears down:

```bash
# DB-tier only (fastest, no sandbox build)
docker-compose/test-runner.sh --tier db

# Sandbox-tier (builds sandbox images)
docker-compose/test-runner.sh --tier sandbox

# Agent-tier (requires Claude credentials)
CLAUDE_CODE_OAUTH_TOKEN=... docker-compose/test-runner.sh --tier agent

# All tiers
CLAUDE_CODE_OAUTH_TOKEN=... docker-compose/test-runner.sh --tier all

# Single test file
docker-compose/test-runner.sh --test task_crud

# Skip build, keep running for debugging
docker-compose/test-runner.sh --skip-build --keep-running --tier db
```

**Via Taskfile:**

```bash
task test:compose:db       # DB tier
task test:compose:sandbox  # Sandbox tier
task test:compose:all      # All tiers
task test:compose -- --test task_crud --keep-running  # Custom flags
```

**Manual** (when backend is already running):

```bash
# DB-tier tests
MOATLESS_API_URL=http://localhost:8080 cargo test -p moatless-integration-tests -- --nocapture

# Sandbox tests
SANDBOX_E2E=1 MOATLESS_API_URL=http://localhost:8080 cargo test -p moatless-integration-tests -- --ignored --nocapture

# Single test
cargo test -p moatless-integration-tests --test task_crud -- --nocapture
```

### Writing a New Rust Integration Test

1. Create `crates/integration-tests/tests/<name>.rs`
2. Use the shared helpers:

```rust
use integration_tests::{test_client, seed_test_users, reset_database};

#[tokio::test]
async fn my_test() {
    reset_database();
    seed_test_users().await;

    let client = test_client();
    // ... test API calls via client.api()
}
```

3. For sandbox/agent tests, add `#[ignore]` and document the env gate:

```rust
#[tokio::test]
#[ignore] // Requires SANDBOX_E2E=1
async fn my_sandbox_test() {
    // Use sandbox_helpers for polling
    use integration_tests::sandbox_helpers;
    // ...
}
```

---

## Playwright UI E2E Tests

### Structure

```
tests/ui/
  playwright.config.ts       # Config: Chromium, 180s timeout, 2 workers
  specs/
    smoke.spec.ts            # App loads, endpoints accessible
    task-creation.spec.ts    # Create task via API, verify in UI
    navigation.spec.ts       # Route navigation, direct URL access
  fixtures/
    ui-fixtures.ts           # Custom fixtures: api, testRepoId
tests/shared/
  api-client.ts              # Type-safe API client (fetch-based, ~100 methods)
  wait-helpers.ts            # Polling helpers (workspace ready, agent idle)
  sse-collector.ts           # NDJSON event stream collector
```

### Configuration

- **Browser:** Chromium only
- **Test timeout:** 180s
- **Expect timeout:** 15s
- **Workers:** 2 (CI: 1)
- **Base URL:** `DOCKER_COMPOSE_URL` or `http://localhost:${BACKEND_PORT:-8080}`
- **Artifacts:** Screenshots on failure, videos retained on failure, traces on first retry

### Custom Fixtures

The `ui-fixtures.ts` provides:

- **`api`** — `ApiClient` instance pointed at the backend. Performs a health check on setup (throws if backend is down).
- **`testRepoId`** — Template repository ID. Auto-creates one if none exists.

Usage in tests:

```typescript
import { test, expect } from "../fixtures/ui-fixtures";

test("my test", async ({ page, api, testRepoId }) => {
  const task = await api.createVibeTask({
    templateId: testRepoId,
    name: "Test Task",
    message: "Do something",
  });

  await page.goto(`/tasks/${task.taskId}`);
  await page.waitForLoadState("networkidle");
  await expect(page.locator("#root")).toBeVisible();

  // Cleanup
  await api.stopTask(task.taskId);
  await api.deleteTask(task.taskId);
});
```

### Running Playwright UI Tests

Requires the backend to be running (via Docker Compose or `dev-docker.sh`).

```bash
# API suite (chains) — note: test:e2e targets tests/api, not the UI
bun run test:e2e

# UI suite (browser)
bun run test:e2e:ui

# Via Taskfile
task test:ui
```

### Writing a New Playwright Test

1. Create `tests/ui/specs/<name>.spec.ts`
2. Import from fixtures:

```typescript
import { test, expect } from "../fixtures/ui-fixtures";

test.describe("Feature Name", () => {
  test("scenario description", async ({ page, api, testRepoId }) => {
    // Setup via API (faster than UI interaction)
    // Navigate with page.goto()
    // Assert with expect()
    // Cleanup via API
  });
});
```

---

## Cluster target (env-agnostic suite)

`tests/cluster/` runs against a **live deployment** — local-k8s or remote — as a
black-box HTTP client. The same suite runs everywhere; only two env vars change.

| Var | local-k8s | remote |
|-----|-----------|--------|
| `MOATLESS_BASE_URL` | `http://moatless.localhost` | `https://moatless.dev` |
| `API_KEY` | the cluster's `TEST_API_KEY` (`mvk_…`) | same |

```bash
MOATLESS_BASE_URL=… API_KEY=mvk_… bun run test:e2e:cluster              # smoke + lifecycle
MOATLESS_BASE_URL=… API_KEY=mvk_… E2E_DEPTH=full bun run test:e2e:cluster   # + agent tier
```

**Auth** is one `test-api-user` (admin) Bearer key. The backend mints it from
`TEST_API_KEY`; add that key to the cluster's Secret (local: `k8s/local/README.md`
step 3 — the chart wires it as an optional env). No login form, no ForwardAuth.

**Depth** (`E2E_DEPTH`):
- unset / `lifecycle` — smoke + lifecycle: auth, task CRUD, sandbox reaches
  ready, preview serves, events over SSE. Deterministic, free.
- `full` — also the agent tier: the agent edits the page to a unique per-run
  marker and the test asserts the **preview renders it**. Needs Claude creds in
  the cluster; the agent spec self-skips when unset.

**Isolation**: no DB reset. Tasks are named `e2e-<runId>-<slug>`; each test
deletes its own, and teardown sweeps this run's prefix. Assert only on resources
you created — never global counts.

**Where it runs**: developers point it at their local-k8s; CI runs it against
remote post-deploy (smoke + lifecycle) and nightly (full), via
`.github/workflows/cluster-e2e.yml`. Compose stays the per-PR gate.

Debugging a red cluster run (pods, logs, metrics, event-delivery mode) → the
`kubernetes-testing` skill.

---

## Environment Setup

### Docker Compose Test Runner (recommended)

`docker-compose/test-runner.sh` handles the full lifecycle:

1. Builds backend Docker image (and sandbox images if needed)
2. Stops existing containers, removes volumes
3. Starts `postgres`, `redis`, `backend` with CI overlay (auth disabled)
4. Polls `/health` until ready (60s max)
5. Resets database (truncates all tables)
6. Runs tests, captures output to `test-logs/`
7. Captures compose logs
8. Tears down (unless `--keep-running`)

**Flags:**

| Flag | Description |
|------|-------------|
| `--tier <db\|sandbox\|agent\|all>` | Test tier (default: `db`) |
| `--test <name>` | Single test file |
| `--skip-build` | Reuse existing Docker images |
| `--keep-running` | Don't tear down (for debugging) |
| `--verbose` | Set `RUST_LOG=debug` on backend |

**Environment variables:**

| Variable | Required for | Default |
|----------|-------------|---------|
| `TASK_TOKEN_SECRET` | All | Auto-generated |
| `DATA_DIR` | All | `$HOME/.moatless/data` |
| `CLAUDE_CODE_OAUTH_TOKEN` | agent tier | - |
| `ANTHROPIC_API_KEY` | agent tier | - |
| `GITHUB_PAT` | repo clone tests | - |

### Manual Setup

```bash
# Start backend + dependencies
./docker-compose/dev-docker.sh --no-logs

# Then run tests separately:
cargo test -p moatless-integration-tests -- --nocapture    # Rust tests
bun run test:e2e                                   # Playwright tests
```

### After `--keep-running`

```bash
# Inspect logs
docker compose -f docker-compose/docker-compose.yml -f docker-compose/docker-compose.ci.yml logs backend

# Tear down manually
docker compose -f docker-compose/docker-compose.yml -f docker-compose/docker-compose.ci.yml down --remove-orphans
```

---

## Debugging Test Failures

**Rust tests:**
- Check `test-logs/test-<tier>-<timestamp>.log` for test output
- Check `test-logs/compose-<timestamp>.log` for backend logs
- Use `--keep-running` + `--verbose` to inspect live

**Playwright tests:**
- HTML report: `test-results/ui/`
- Screenshots captured on failure

**Common issues:**
- Backend not healthy: check `docker compose logs backend`
- Tests can't connect: verify `MOATLESS_API_URL` is set correctly
- Sandbox tests fail: ensure sandbox image is built (`docker/build-sandbox.sh --local`)
- Agent tests fail: check `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` is set
