# Backend Testing Guidelines

## Critical Testing Principles

**ALWAYS write unit tests** for any backend component. Unit tests are mandatory for all business logic, error handling, and state management.

**Decision Matrix**:
1. Write UNIT tests for all business logic (mandatory)
2. ADD integration tests if testing real system interactions is critical
   - Narrow scope: Single component + external system
   - Broad scope: Multiple components working together

## Test Levels

### UNIT TESTS (*.test.ts or *.unit.test.ts)
- **Purpose**: Test business logic in isolation
- **Mock**: ALL external dependencies (filesystem, network, databases, child processes, other services/modules, time-based operations)
- **Use for**: Algorithms, validation, error handling, state management, configuration parsing

### INTEGRATION TESTS (*.integration.test.ts)
- **Purpose**: Test real system interactions
- **Two scopes**:

**Narrow Integration**: One component + real external system
- DockerEnvironment + real Docker
- GitService + real Git
- Storage + real filesystem
- Database service + real database
- **Mock**: Other app components only

**Broad Integration**: Multiple components working together
- Complete workflows: API → Service → External System → Results
- User journeys: Register → Login → Access resources
- **Mock**: Only external third-party services

**Key distinction**:
- Narrow: "Does DockerEnvironment work with Docker?"
- Broad: "Does the complete task execution workflow function?"

## Data Directory Management

```typescript
import { getDataDir } from '../utils/data-dir'
const dataDir = getDataDir() // Auto-isolated in tests
// NEVER access process.env.DATA_DIR directly
```

## Testing Patterns

### Pattern 1: Service with External Dependencies

```typescript
// ALWAYS: Unit tests (mandatory)
describe('TaskService - Unit Tests', () => {
  let mockStorage: MockedStorage
  let service: TaskService

  beforeEach(() => {
    mockStorage = createMockStorage()
    service = new TaskService(mockStorage)
  })

  it('should validate task parameters', () => {
    // Test pure logic with mocks
  })

  it('should handle storage errors gracefully', () => {
    // Test error handling
  })
})

// OPTIONAL: Integration tests
describe('TaskService - Integration Tests', () => {
  let service: TaskService
  let tempDir: string

  beforeEach(async () => {
    tempDir = await createTempDir()
    const storage = new FileStorage(tempDir) // Real storage
    service = new TaskService(storage)
  })

  it('should persist task to storage', async () => {
    // Test with real file system
  })
})
```

### Pattern 2: External System Integration

```typescript
// Unit tests (mandatory)
describe('DockerEnvironment - Unit Tests', () => {
  beforeEach(() => {
    vi.mock('node:child_process', () => ({
      execSync: vi.fn(cmd => 'mocked response')
    }))
  })

  it('should handle state transitions correctly', () => {
    // Test business logic with mocked Docker
  })
})

// Narrow integration tests
describe('DockerEnvironment - Integration Tests', () => {
  let env: DockerEnvironment

  afterEach(async () => {
    await env.teardown() // Clean real resources
  })

  it('should create real Docker container', async () => {
    await env.setup()
    const result = await execAsync('docker ps')
    expect(result).toContain(env.getContainerName())
  })

  it('should recover from container errors', async () => {
    await env.setup()
    await execAsync(`docker kill ${env.getContainerName()}`)
    await env.ensureContainerReady() // Should self-heal
    const result = await env.runCommand('echo "OK"')
    expect(result.exitCode).toBe(0)
  })
})

// Broad integration tests
describe('Task Execution - Integration Tests (Broad)', () => {
  it('should execute task from API to completion', async () => {
    const repo = await api.post('/repositories', {
      codeExecution: { type: 'docker' }
    })
    const task = await api.post('/tasks', {
      repositoryId: repo.id,
      message: 'Create index.js'
    })
    await waitForTaskCompletion(task.id)
    const file = await api.get(`/tasks/${task.id}/files/index.js`)
    expect(file).toBeDefined()
  })
})
```

## Anti-Patterns

1. **Missing unit tests**: Always have unit tests, even with integration tests
2. **Fake integration tests**: Test successful integration, not expected failures
   ```typescript
   // WRONG: Testing failure without real system
   it('should fail when container missing', () => {})
   
   // RIGHT: Test real integration
   it('should execute in real container', () => {})
   ```
3. **Over-mocking in integration tests**: Use real implementations for what you're testing

## Quick Decision Guide

1. **Always**: Write unit tests
2. **Single component + external system?** → Narrow integration test
3. **Multiple components workflow?** → Broad integration test

### Examples by Component:

**DockerEnvironment**:
- Unit: Mock Docker, test logic
- Narrow integration: Real Docker container operations
- Broad integration: API → Task → Docker → Results

**API Routes**:
- Unit: Test validation logic
- Narrow integration: Route + real database
- Broad integration: Complete user workflows

## Special Cases

- **Server/Process Management**: Use broad integration tests
- **SSE/WebSocket**: Mock in unit, real connections in integration
- **Database**: Mock in unit, test database in integration
- **External APIs**: Mock in unit, use stubs/sandboxes in integration

## Remember

- Unit tests are MANDATORY
- Integration tests verify real interactions (narrow or broad scope)
- Choose by scope, not performance
- Mock appropriately based on test level
- Name files correctly: `*.test.ts` (unit) or `*.integration.test.ts` (integration)