# E2E Testing Strategy

This document describes the end-to-end testing strategy for Moatless Vibe, covering test environments, running tests, debugging, and best practices.

## Overview

### Test Pyramid

The project follows a test pyramid approach:

```
        ╱╲
       ╱  ╲      E2E Tests (Playwright)
      ╱────╲     - Full system integration
     ╱      ╲    - Docker workflow tests
    ╱────────╲   - UI workflow tests
   ╱          ╲
  ╱  Integration ╲  Integration Tests (Vitest)
 ╱    Tests      ╲  - API endpoint tests
╱────────────────╲  - Service integration
╱                  ╲
╱    Unit Tests      ╲  Unit Tests (Vitest)
╱                      ╲  - Pure functions
╱──────────────────────╲  - Component rendering
```

### When to Use Each Test Type

| Test Type | Use When | Location |
|-----------|----------|----------|
| **Unit Tests** | Testing pure functions, utilities, isolated components | `**/__tests__/*.test.ts` |
| **Integration Tests** | Testing API endpoints, service interactions | `**/__tests__/*.integration.test.ts` |
| **E2E Tests (Spawned)** | Testing full UI workflows with fresh server | `tests/e2e/*.spec.ts` |
| **E2E Tests (Docker)** | Testing Docker container workflows against docker-compose | `tests/e2e/docker/specs/*.spec.ts` |

## Test Environments

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     E2E TEST ENVIRONMENTS                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────────────┐    ┌──────────────────────┐           │
│  │  Spawned Backend     │    │   Docker-Compose      │           │
│  │  (playwright.config) │    │   (playwright.docker) │           │
│  ├──────────────────────┤    ├──────────────────────┤           │
│  │ • Fresh server/test  │    │ • Shared environment  │           │
│  │ • Isolated data      │    │ • Real containers     │           │
│  │ • Fast setup         │    │ • Full integration    │           │
│  │ • UI + API tests     │    │ • API tests primarily │           │
│  │ • Chromium browser   │    │ • Single worker       │           │
│  └──────────────────────┘    └──────────────────────┘           │
│                                                                  │
│  Config: playwright.config.ts   Config: playwright.docker.config │
│  Setup: global-setup.ts         Setup: None (external)           │
│  Cleanup: global-teardown.ts    Cleanup: Fixture-based           │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Spawned Backend (playwright.config.ts)

- **Purpose**: UI component tests, isolated workflow tests
- **Server**: Fresh backend spawned for each test run via global-setup.ts
- **Data**: Temporary directory, cleaned after tests
- **Browsers**: Chromium (configurable)
- **Parallelization**: Full parallel execution

### Docker-Compose (playwright.docker.config.ts)

- **Purpose**: Full Docker container integration tests
- **Server**: Running docker-compose environment (external)
- **Data**: Shared PostgreSQL database, Redis
- **Browsers**: N/A (API-first tests)
- **Parallelization**: Sequential (single worker)

## Running Tests

### Prerequisites

1. **Node.js 22+** and **Bun** package manager
2. **Docker** (for Docker E2E tests)
3. **Claude Authentication**: `CLAUDE_CODE_OAUTH_TOKEN` environment variable

### Spawned Backend Tests

```bash
# Run the API E2E suite (tests/api, no browser) — the canonical entrypoint
bun run test:e2e

# Same, explicit alias
bun run test:e2e:api

# Run the browser (UI) suite (tests/ui, Chromium)
bun run test:e2e:ui

# Run a specific chain
bun run test:e2e -- tests/api/chains/task-crud.spec.ts
```

These run against a stack you start yourself (`cd docker-compose && ./dev-docker.sh`).

### Containerized (Docker) Tests

The `test:e2e:docker*` scripts run the suites inside a container on the compose
network (build stack → wait for health → run → tear down), via
`docker-compose/playwright-runner.sh`:

```bash
# API suite (default)
bun run test:e2e:docker            # or: task test:compose:e2e

# Browser suite (builds the frontend image)
bun run test:e2e:docker:ui         # or: task test:compose:e2e -- --suite ui

# Both suites
bun run test:e2e:docker:all        # or: task test:compose:e2e -- --suite all

# Forward args to Playwright (e.g. a single chain)
bun run test:e2e:docker -- tests/api/chains/task-crud.spec.ts
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `CLAUDE_CODE_OAUTH_TOKEN` | Claude authentication token | Required for agent tests |
| `DOCKER_COMPOSE_URL` | Backend URL for Docker tests | `http://localhost:8000` |
| `DEBUG_SSE` | Enable SSE debug logging | `false` |
| `E2E_BASE_URL` | Base URL for spawned tests | `http://127.0.0.1:3002` |

## Debugging & Troubleshooting

### Reading Container Logs

```bash
# View sandbox container logs (replace <task-id-prefix> with first 8 chars of task ID)
docker logs moatless-moatable-<task-id-prefix>

# Stream logs in real-time
docker logs -f moatless-moatable-<task-id-prefix>

# View last 100 lines
docker logs --tail 100 moatless-moatable-<task-id-prefix>

# Save logs to file
docker logs moatless-moatable-<task-id-prefix> > /tmp/sandbox.log 2>&1

# List all moatless containers
docker ps -a --filter "name=moatless-"
```

### Playwright Trace Viewer

When tests fail, Playwright captures traces that can be analyzed:

```bash
# Open trace file (after test failure)
npx playwright show-trace test-results/docker-e2e/trace.zip

# Enable trace on all tests (not just retries)
bun run test:e2e:docker -- --trace on

# View HTML report
npx playwright show-report test-results/docker-e2e
```

### Common Failure Patterns

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| Workspace timeout | Slow image pull | Pre-pull image: `docker pull aorwall/moatless-vibe-sandbox-js:latest` |
| Agent never completes | Claude auth issue | Verify `CLAUDE_CODE_OAUTH_TOKEN` is set |
| Preview not accessible | Port mapping issue | Check docker port mapping: `docker port <container>` |
| SSE events missing | Redis not running | Verify redis container: `docker ps \| grep redis` |
| Connection refused | Backend not started | Start docker-compose: `./docker-compose/dev-docker.sh` |
| Container not found | Container was removed | Check if task was deleted, re-run test |

### Inspecting Test State

```bash
# Check container state
docker inspect moatless-moatable-<task-id> --format='{{.State.Status}}'

# Check container network
docker inspect moatless-moatable-<task-id> --format='{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}'

# Check container port mappings
docker port moatless-moatable-<task-id>

# Check backend health
curl http://localhost:8000/health

# List recent tasks via API
curl http://localhost:8000/api/v1/tasks | jq '.items[:5]'
```

## Writing Tests

### Test Structure Pattern

```typescript
import { test, expect } from "../fixtures/docker-test-fixtures";
import { waitForWorkspaceReady, waitForAgentIdleViaSSE } from "../helpers";

test.describe("Feature Name", () => {
  // Configure serial execution if tests depend on each other
  test.describe.configure({ mode: "serial" });

  test("should do something", async ({ api, sse, testTask }) => {
    // Arrange - trigger provisioning
    await api.getFiles(testTask.taskId);
    await waitForWorkspaceReady(api, testTask.taskId, { timeout: 180000 });

    // Act - perform the action
    await api.sendMessage(testTask.taskId, "Do something");

    // Wait - use wait helpers for async operations
    const completed = await waitForAgentIdleViaSSE(sse, api, testTask.taskId, {
      timeout: 120000,
    });

    // Assert - verify results
    expect(completed).toBe(true);

    // Verify via API
    const content = await api.getFileContent(testTask.taskId, "result.txt");
    expect(content).toContain("expected");

    // Verify events (optional)
    const events = sse.getEventsForTask(testTask.taskId);
    expect(events).toContainEqual(
      expect.objectContaining({ type: "agent", subType: "completed" })
    );
  });
});
```

### Using Fixtures

The Docker E2E tests provide several fixtures:

```typescript
test("example", async ({ api, sse, testRepository, testTask }) => {
  // api: ApiClient - Type-safe API wrapper
  // sse: SSEEventCollector - SSE event collection
  // testRepository: Auto-created repository with cleanup
  // testTask: Auto-created task with cleanup

  // Fixtures handle cleanup automatically!
});
```

### Best Practices

1. **Use fixtures for resource lifecycle** - Don't manually create/delete repositories and tasks
2. **Always wait for async operations** - Use `waitForWorkspaceReady`, `waitForPreviewReady`, etc.
3. **Verify via both API and SSE events** - Provides redundancy if one fails
4. **Add console.log for debugging visibility** - Helps when analyzing failures
5. **Set appropriate timeouts** - Container operations need longer timeouts
6. **Save container logs on failure** - Critical for debugging agent issues

### Anti-Patterns

```typescript
// BAD: Manual cleanup without fixtures
let taskId: string;
test.afterEach(async () => {
  await api.deleteTask(taskId);
});

// GOOD: Use fixtures
test("example", async ({ testTask }) => {
  // testTask.cleanup() is called automatically
});

// BAD: No wait for async operation
await api.sendMessage(taskId, "do something");
const content = await api.getFileContent(taskId, "file.txt"); // May fail!

// GOOD: Wait for completion
await api.sendMessage(taskId, "do something");
await waitForAgentIdleViaSSE(sse, api, taskId);
const content = await api.getFileContent(taskId, "file.txt");
```

## Test File Organization

```
tests/e2e/
├── helpers/                    # Existing helpers for spawned tests
│   ├── global-setup.ts         # Server spawn setup
│   ├── global-teardown.ts      # Server cleanup
│   └── test-server.ts          # Test utilities
├── smoke.spec.ts               # Existing smoke tests
├── ui-task-complete-workflow.spec.ts  # Existing UI test
│
├── docker/                     # Docker-compose tests
│   ├── helpers/                # Modular test helpers
│   │   ├── api-client.ts       # Type-safe API wrapper
│   │   ├── sse-collector.ts    # SSE event collection
│   │   ├── wait-helpers.ts     # Polling/wait utilities
│   │   ├── docker-helpers.ts   # Docker-specific operations
│   │   └── index.ts            # Re-exports
│   │
│   ├── fixtures/               # Playwright fixtures
│   │   └── docker-test-fixtures.ts  # Task/repo lifecycle
│   │
│   └── specs/                  # Test specifications
│       └── docker-workflow.spec.ts  # Main E2E workflow
```

## CI/CD Integration

### GitHub Actions Workflow

```yaml
# .github/workflows/e2e-docker.yml
name: Docker E2E Tests

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  e2e-docker:
    runs-on: ubuntu-latest
    services:
      docker:
        image: docker:dind
        options: --privileged

    steps:
      - uses: actions/checkout@v4

      - name: Setup Bun
        uses: oven-sh/setup-bun@v1
        with:
          bun-version: latest

      - name: Install dependencies
        run: bun install

      - name: Start Docker Compose
        run: |
          cd docker-compose
          docker compose up -d
          # Wait for services to be healthy
          sleep 30
          curl --retry 10 --retry-delay 5 http://localhost:8000/health

      - name: Run E2E Tests
        run: bun run test:e2e:docker
        env:
          CLAUDE_CODE_OAUTH_TOKEN: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}

      - name: Upload artifacts on failure
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: e2e-test-results
          path: |
            test-results/docker-e2e/
            /tmp/e2e-*.log
```

### Artifact Collection

On test failure, the following artifacts are collected:
- HTML test report
- JSON test results
- Playwright traces (on retry)
- Screenshots (on failure)
- Container logs (saved to /tmp)

## API Reference

### ApiClient

```typescript
class ApiClient {
  // Repository operations
  createRepository(config: CreateRepositoryRequest): Promise<Repository>;
  getRepository(id: string): Promise<Repository>;
  deleteRepository(id: string): Promise<void>;
  listRepositories(): Promise<Repository[]>;

  // Task operations
  createTask(config: CreateTaskRequest): Promise<Task>;
  getTask(taskId: string): Promise<Task>;
  deleteTask(taskId: string): Promise<void>;
  stopTask(taskId: string): Promise<void>;
  sendMessage(taskId: string, message: string): Promise<void>;

  // Workspace operations
  getWorkspaceStatus(taskId: string): Promise<WorkspaceStatus>;
  getFiles(taskId: string, path?: string): Promise<FileListResponse>;
  getFileContent(taskId: string, path: string): Promise<string>;
  writeFile(taskId: string, path: string, content: string): Promise<void>;
  getGitDiff(taskId: string): Promise<GitDiffResponse>;

  // Preview operations
  getPreviewStatus(taskId: string): Promise<PreviewStatus>;
  startPreview(taskId: string): Promise<void>;
  stopPreview(taskId: string): Promise<void>;
}
```

### SSEEventCollector

```typescript
class SSEEventCollector {
  start(url: string): Promise<void>;
  stop(): void;

  getAllEvents(): SSEEvent[];
  getEventsForTask(taskId: string): SSEEvent[];

  waitForEvent(options: {
    type: string;
    subType?: string;
    taskId?: string;
    timeout?: number;
  }): Promise<SSEEvent | null>;

  verifyTaskLifecycle(taskId: string): TaskLifecycleVerification;
  getEventSummary(taskId?: string): Record<string, number>;
}
```

### Wait Helpers

```typescript
// Wait for workspace to be ready
waitForWorkspaceReady(api, taskId, options?): Promise<boolean>;

// Wait for preview server
waitForPreviewReady(api, taskId, options?): Promise<PreviewStatus | null>;

// Wait for agent completion
waitForAgentComplete(api, taskId, options?): Promise<boolean>;

// Wait for agent via SSE (preferred)
waitForAgentIdleViaSSE(sse, api, taskId, options?): Promise<boolean>;

// Wait for specific workspace status
waitForWorkspaceStatus(api, taskId, status, options?): Promise<WorkspaceStatus | null>;

// Wait for file to exist
waitForFile(api, taskId, filePath, options?): Promise<boolean>;

// Generic condition wait
waitForCondition(condition: () => Promise<boolean>, options?): Promise<boolean>;
```

### Docker Helpers

```typescript
// Availability checks
isDockerAvailable(): Promise<boolean>;
isDockerComposeHealthy(backendUrl?): Promise<HealthStatus>;

// Container operations
getContainerState(name: string): Promise<ContainerState | null>;
getContainerLogs(name: string, tail?: number): Promise<string>;
saveContainerLogs(name: string, path: string, options?): Promise<void>;
getContainerNetworks(name: string): Promise<ContainerNetwork[]>;
verifyContainerNetwork(name: string, network: string): Promise<boolean>;
getContainerPort(name: string, port: number): Promise<number | null>;

// Container naming
getContainerNameForTask(taskId: string): string;
listMoatlessContainers(): Promise<string[]>;

// Cleanup
removeContainer(name: string): Promise<boolean>;
cleanupTestContainers(): Promise<number>;
```

## Future Enhancements

1. **UI Tests on Docker**: Add browser-based UI tests against docker-compose
2. **Kubernetes E2E**: Separate config for K8s testing
3. **Performance Benchmarks**: Measure and track test execution time
4. **Visual Regression**: Screenshot comparison for UI tests
5. **Parallel Docker Tests**: Multiple workers with isolated containers
