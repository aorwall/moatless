# Backend Testing Guidelines

This document provides specific testing guidelines for the backend application, complementing the project-wide testing guidelines in CLAUDE.md.

## Data Directory Management

### Backend Data Directory Pattern

The backend uses a centralized data directory management system to ensure tests use isolated temporary directories while production code uses the correct persistent directories.

#### Common Function Usage

All backend code should use the `getDataDir()` function instead of directly accessing `process.env.DATA_DIR`:

```typescript
import { getDataDir } from '../utils/data-dir.js'

// ✅ GOOD: Use the common function
const dataDir = getDataDir()
const tasksDir = path.join(dataDir, 'tasks')

// ❌ BAD: Direct environment variable access
const dataDir = process.env.DATA_DIR || './data'
```

#### Test Setup

The test setup (`apps/backend/test/setup.ts`) automatically:
1. Creates a unique temporary directory for each test run
2. Sets `process.env.DATA_DIR` to point to this temporary directory
3. Cleans up after each test and after all tests complete

```typescript
// Test setup handles this automatically
beforeAll(async () => {
  tempDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-test-data-'))
  process.env.DATA_DIR = tempDataDir
})

beforeEach(async () => {
  // Clean and recreate for each test
  await fs.rm(tempDataDir, { recursive: true, force: true })
  await fs.mkdir(tempDataDir, { recursive: true })
})
```

#### Production vs Test Behavior

- **Production**: `getDataDir()` returns `~/.moatless` (or `DATA_DIR` if set)
- **Tests**: `getDataDir()` returns the temporary directory set by test setup
- **Custom**: Set `DATA_DIR` environment variable to override

## Test Organization & Naming Conventions

### Test File Naming

Tests must follow strict naming conventions to ensure proper categorization and execution in different test suites:

#### Unit Tests (FAST - <200ms)
- **Pattern**: `*.test.ts` or `*.unit.test.ts`
- **Location**: `__tests__` directories co-located with source code
- **Examples**:
  - `utils/__tests__/logger.test.ts`
  - `services/__tests__/storage.test.ts`
  - `services/__tests__/task.unit.test.ts`

#### Integration Tests (MODERATE - 200ms-2s)
- **Pattern**: `*.integration.test.ts`
- **Location**: `__tests__` directories co-located with source code
- **Examples**:
  - `routes/__tests__/tasks.integration.test.ts`
  - `services/__tests__/workspace.integration.test.ts`
  - `services/__tests__/claude-code.integration.test.ts`

#### E2E Tests (SLOW - >2s)
- **Pattern**: `*.e2e.test.ts`
- **Location**: `__tests__` directories co-located with source code
- **Examples**:
  - `routes/__tests__/complete-workflow.e2e.test.ts`
  - `routes/__tests__/repositories.e2e.test.ts`
  - `routes/__tests__/server-lifecycle.e2e.test.ts`

#### Server/Process Tests (SLOW - >2s)
- **Pattern**: `*server*.test.ts`, `*shutdown*.test.ts`, `*lifecycle*.test.ts`
- **Location**: Appropriate `__tests__` directories
- **Examples**:
  - `services/__tests__/preview-environment-shutdown.test.ts`
  - `__tests__/graceful-shutdown.test.ts`
  - `routes/__tests__/agent-lifecycle.test.ts`

### Test Categories & Performance Tiers

#### Test Classification Decision Tree
```
Does the test perform any I/O operations?
├─ No → FAST Test (use *.test.ts)
└─ Yes → What type of I/O?
    ├─ File system only → MODERATE Test (use *.integration.test.ts)
    ├─ Network calls → MODERATE Test (use *.integration.test.ts)
    ├─ Process spawning → SLOW Test (use *.e2e.test.ts)
    └─ Real server startup → SLOW Test (use *.e2e.test.ts)
```

#### 1. FAST Tests (<200ms) - Run on every push

**Concrete Criteria - Must meet ALL**:
- Zero file system operations (no fs.readFile, fs.writeFile, etc.)
- Zero network calls (no fetch, axios, http requests)
- Zero process spawning (no child_process, execa)
- All external dependencies are mocked
- No setTimeout/setInterval/real timers
- No database connections

**Examples of FAST tests**:
- Service unit tests with mocked storage
- Utility function tests (pure functions)
- Business logic tests with mocked dependencies
- Data transformation tests

```typescript
// Example: Fast unit test
describe('TaskService - Unit Tests', () => {
  let taskService: TaskService
  let mockStorage: vi.Mocked<StorageService>
  let mockWorkspace: vi.Mocked<WorkspaceService>

  beforeEach(() => {
    mockStorage = createMockStorageService()
    mockWorkspace = createMockWorkspaceService()
    taskService = new TaskService(mockStorage, mockWorkspace)
  })

  it('should create task metadata', async () => {
    // Pure business logic test with mocked dependencies
    const result = await taskService.createTask({ ... })
    expect(result).toBeDefined()
    expect(mockStorage.save).toHaveBeenCalledOnce()
  })
})
```

#### 2. MODERATE Tests (200ms-2s) - Run in CI

**Concrete Criteria - Must meet ALL**:
- Performs file system operations OR
- Makes local network calls (localhost only) OR
- Uses real database connections OR
- Executes git commands
- Does NOT spawn long-running processes
- Does NOT start real servers
- Completes within 2 seconds

**Examples of MODERATE tests**:
- Route handler tests with real app instance
- Service tests with real file system access
- Database CRUD operation tests
- Git command execution tests (add, commit, diff)

```typescript
// Example: Integration test
describe('Task Routes - Integration Tests', () => {
  let app: ReturnType<typeof build>

  beforeEach(async () => {
    app = await build() // Real app with test database
  })

  it('should create and retrieve task', async () => {
    const response = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: { ... }
    })
    expect(response.statusCode).toBe(201)
  })
})
```

#### 3. SLOW Tests (>2s) - Run separately or nightly

**Concrete Criteria - ANY of these**:
- Starts a real HTTP/WebSocket server
- Spawns child processes that run >2 seconds
- Makes external network calls (non-localhost)
- Tests complete user workflows end-to-end
- Requires multiple service interactions
- Tests process lifecycle (startup/shutdown)
- Establishes persistent connections (SSE/WebSocket)

**Examples of SLOW tests**:
- Server startup and graceful shutdown tests
- Complete task execution workflow tests
- Preview environment lifecycle tests
- Real agent process spawning tests
- SSE/WebSocket connection establishment tests

```typescript
// Example: E2E test
describe('Complete Workflow - E2E', () => {
  let server: Server
  let port: number

  beforeEach(async () => {
    port = await getRandomPort()
    server = await startRealServer(port)
  })

  afterEach(async () => {
    await server.close()
  })

  it('should handle complete task workflow', async () => {
    // Test with real server, real file system, real processes
  })
})
```

## Test Command Usage

```bash
# Development - Fast feedback loop (unit tests only)
pnpm test:unit

# Pre-push hook - Essential coverage (<30s)
pnpm test:fast

# CI Pipeline - Full integration testing
pnpm test

# Nightly/Release - Complete validation including E2E
pnpm test:all

# With coverage reporting
pnpm test:coverage

# Interactive UI mode
pnpm vitest --ui
```

## Backend-Specific Testing Patterns

### 1. Service Testing

#### Unit Tests (FAST)
Mock all dependencies and test business logic:

```typescript
// services/__tests__/task.unit.test.ts
describe('TaskService - Unit Tests', () => {
  let taskService: TaskService
  let mockDeps: MockedDependencies

  beforeEach(() => {
    mockDeps = createMockDependencies()
    taskService = new TaskService(mockDeps)
  })

  it('should validate task creation parameters', () => {
    // Test pure business logic
  })
})
```

#### Integration Tests (MODERATE)
Test with real file system but mock external services:

```typescript
// services/__tests__/task.integration.test.ts
describe('TaskService - Integration Tests', () => {
  let taskService: TaskService
  let tempDir: string

  beforeEach(async () => {
    tempDir = await createTempDir()
    taskService = await createTaskService({ dataDir: tempDir })
  })

  afterEach(async () => {
    await cleanupTempDir(tempDir)
  })

  it('should persist task to file system', async () => {
    // Test with real file operations
  })
})
```

### 2. Route Testing

#### Integration Tests (MODERATE)
Test API endpoints with mocked services:

```typescript
// routes/__tests__/tasks.integration.test.ts
describe('Task Routes - Integration Tests', () => {
  let app: HonoApp
  let mockTaskService: MockedTaskService

  beforeEach(async () => {
    mockTaskService = createMockTaskService()
    app = await buildApp({ taskService: mockTaskService })
  })

  it('should handle task creation request', async () => {
    const res = await app.request('/api/v1/tasks', {
      method: 'POST',
      body: JSON.stringify({ ... })
    })
    expect(res.status).toBe(201)
  })
})
```

#### E2E Tests (SLOW)
Test complete flows with real services:

```typescript
// routes/__tests__/tasks.e2e.test.ts
describe('Task Routes - E2E Tests', () => {
  let server: Server
  let client: TestClient

  beforeAll(async () => {
    server = await startTestServer()
    client = createTestClient(server.port)
  })

  afterAll(async () => {
    await server.close()
  })

  it('should handle complete task lifecycle', async () => {
    // Create task
    const task = await client.createTask({ ... })
    
    // Start task
    await client.startTask(task.id)
    
    // Wait for completion
    await client.waitForTaskCompletion(task.id)
    
    // Verify results
    const result = await client.getTaskResult(task.id)
    expect(result.status).toBe('completed')
  })
})
```

### 3. Process & Server Testing

Always mark these as SLOW tests:

```typescript
// services/__tests__/preview-environment-server.test.ts
describe('Preview Environment Server Tests', () => {
  let service: PreviewEnvironmentService

  beforeEach(async () => {
    service = new PreviewEnvironmentService()
  })

  afterEach(async () => {
    await service.gracefulShutdown()
  })

  it('should start and stop preview server', async () => {
    const server = await service.startServer({ ... })
    expect(server.port).toBeDefined()
    
    await service.stopServer(server.id)
    expect(service.getRunningServers()).toHaveLength(0)
  }, { timeout: 10000 }) // Explicit timeout for server tests
})
```

### 4. SSE Testing

Mock for unit tests, use real connections for E2E:

```typescript
// Unit test - Mock SSE
describe('SSE Manager - Unit Tests', () => {
  it('should track connections', () => {
    const mockController = createMockSSEController()
    sseManager.connect('task-1', mockController)
    expect(sseManager.getConnectionCount('task-1')).toBe(1)
  })
})

// E2E test - Real SSE
describe('SSE Events - E2E Tests', () => {
  it('should stream task events', async () => {
    const eventSource = new EventSource(`${baseUrl}/events/${taskId}`)
    const events = await collectEvents(eventSource, 5)
    expect(events).toHaveLength(5)
  }, { timeout: 30000 })
})
```

## Test Data Management

### Temporary Directories
Always use isolated temp directories for tests:

```typescript
import { createTempDataDir } from '../test/helpers'

beforeEach(async () => {
  process.env.DATA_DIR = await createTempDataDir('test-prefix')
})

afterEach(async () => {
  await cleanupTempDataDir(process.env.DATA_DIR)
})
```

### Git Repository Setup
For tests requiring git repositories:

```typescript
import { setupTestGitRepo } from '../test/git-helpers'

beforeEach(async () => {
  const { repoPath, cleanup } = await setupTestGitRepo({
    files: {
      'README.md': '# Test Repo',
      'src/index.js': 'console.log("test")'
    }
  })
  
  this.repoPath = repoPath
  this.cleanup = cleanup
})

afterEach(async () => {
  await this.cleanup()
})
```

## Performance Optimization

### 1. Use Concurrent Test Setup
When possible, set up test data concurrently:

```typescript
beforeEach(async () => {
  const [tempDir, mockData, testRepo] = await Promise.all([
    createTempDir(),
    generateMockData(),
    setupTestRepo()
  ])
  
  this.testContext = { tempDir, mockData, testRepo }
})
```

### 2. Reuse Test Fixtures
For expensive setup, use `beforeAll`:

```typescript
describe('Repository Tests', () => {
  let sharedRepo: TestRepository

  beforeAll(async () => {
    sharedRepo = await createLargeTestRepository()
  })

  afterAll(async () => {
    await sharedRepo.cleanup()
  })

  // Multiple tests share the same repository
  it('test 1', () => { ... })
  it('test 2', () => { ... })
})
```

### 3. Mock Time-Consuming Operations
For fast tests, mock operations that involve:
- Network calls
- File system operations
- Process spawning
- Database queries

## Common Pitfalls & Solutions

### 1. Port Conflicts
Always use dynamic ports for server tests:

```typescript
import { getRandomPort } from '../test/port-helpers'

const port = await getRandomPort()
const server = await startServer({ port })
```

### 2. Process Cleanup
Always ensure processes are cleaned up:

```typescript
afterEach(async () => {
  // Kill any remaining processes
  await killProcessTree(process.pid)
  
  // Clean up port bindings
  await releasePort(this.port)
})
```

### 3. File System Cleanup
Use try-finally for cleanup:

```typescript
it('should handle file operations', async () => {
  const tempFile = await createTempFile()
  
  try {
    // Test operations
  } finally {
    await fs.unlink(tempFile).catch(() => {})
  }
})
```

### 4. Async Timeout Issues
Set appropriate timeouts for different test types:

```typescript
// Fast unit test - default timeout (5s)
it('should process data quickly', async () => { ... })

// Integration test - moderate timeout
it('should handle file operations', async () => { ... }, 10000)

// E2E test - long timeout
it('should complete workflow', async () => { ... }, 30000)
```

## Debugging Tips

### 1. Enable Console Logging
Tests run with `USE_CONSOLE_LOGGING=true` by default. To disable:

```bash
USE_CONSOLE_LOGGING=false pnpm test
```

### 2. Run Single Test File
```bash
pnpm vitest run services/__tests__/task.test.ts
```

### 3. Run Tests in Watch Mode
```bash
pnpm vitest watch
```

### 4. Debug with Node Inspector
```bash
node --inspect-brk ./node_modules/.bin/vitest run
```

### 5. View Test UI
```bash
pnpm vitest --ui
```

## Migration Guide

To migrate existing tests to follow these guidelines:

1. **Rename test files** according to their category:
   - `preview-environment-shutdown.test.ts` → `preview-environment-shutdown.e2e.test.ts`
   - `claude-code-integration.test.ts` → `claude-code.integration.test.ts`
   - `logger.test.ts` → `logger.unit.test.ts` (optional, can keep as-is)

2. **Update test timeouts** based on category:
   - Unit tests: default (5s)
   - Integration tests: 10-15s
   - E2E tests: 30-60s

3. **Move slow tests** out of the fast test suite:
   - Tests that start real servers
   - Tests that spawn processes
   - Tests with real network calls

4. **Add proper cleanup** to prevent test pollution:
   - Kill spawned processes
   - Clean temp directories
   - Close server connections

5. **Update imports** to use test helpers:
   ```typescript
   import { createTempDataDir, setupTestRepo, getRandomPort } from '../test/helpers'
   ```