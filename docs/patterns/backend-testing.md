# Backend Testing Guidelines

This document provides testing guidelines for the backend application. These instructions are designed for LLMs to understand when and how to write different types of tests.

## Critical Testing Principles

### Test Level Selection Strategy

**ALWAYS write unit tests** for any backend component. Unit tests are mandatory for all business logic, error handling, and state management.

Additionally, you may ALSO write integration or E2E tests when appropriate:

**Mandatory**:
- UNIT tests → Always required for all components

**Optional (choose based on use case)**:
- INTEGRATION tests → When you need to verify real component interactions
- E2E tests → When you need to verify complete user workflows

**Decision Matrix**:
1. Write UNIT tests for all business logic (mandatory)
2. ADD integration tests if testing real system interactions is critical
3. ADD E2E tests if testing complete user journeys is critical

### The Three Test Levels Explained

#### UNIT TESTS (*.test.ts or *.unit.test.ts)
**Purpose**: Test business logic in isolation with all external dependencies mocked

**When to write unit tests**:
- Testing algorithms, calculations, or data transformations
- Testing error handling logic
- Testing state management
- Testing configuration parsing
- Testing validation logic

**What to mock in unit tests**:
- ALL external dependencies (file system, network, databases, child processes)
- ALL other services and modules
- Time-based operations (use fake timers)

**Example scenarios**:
- Testing if DockerEnvironment correctly transitions states
- Testing if TaskService validates input parameters
- Testing if error recovery logic executes correct steps
- Testing if configuration is parsed correctly

#### INTEGRATION TESTS (*.integration.test.ts)
**Purpose**: Test how a single component integrates with its external dependencies

**Scope**: One component + its real external system (NOT the whole application)

**When to write integration tests**:
- Testing if a service correctly uses an external system
- Verifying integration points work as expected
- Testing error handling with real external systems
- Validating configuration and setup of external dependencies

**What to test in integration tests**:
- DockerEnvironment + real Docker (container lifecycle, command execution)
- Repository service + real Git (clone, commit, push operations)
- Storage service + real file system (read, write, permissions)
- Database service + real database (queries, transactions)

**What to mock in integration tests**:
- Other application components (mock the rest of your app)
- Third-party services not being tested
- Nothing about the external system being tested

**Example scenarios**:
- DockerEnvironment creates and manages real containers
- GitService performs real Git operations on repositories
- FileStorage correctly handles file permissions and errors
- DatabaseService handles real connection failures

#### E2E TESTS (*.e2e.test.ts)
**Purpose**: Test complete user workflows through the entire application stack

**Scope**: Multiple components working together to complete user journeys

**When to write E2E tests**:
- Testing complete business workflows
- Verifying critical user paths work correctly
- Testing component interactions in production-like scenarios
- Validating the system works as users expect

**What to test in E2E tests**:
- Complete task execution: API → Task Service → Workspace → Docker → Agent → Results
- Full repository workflow: Create repo → Configure → Create tasks → View results
- User authentication flow: Register → Login → Access protected resources
- Preview server lifecycle: Start server → Make changes → See updates → Stop server

**What to mock in E2E tests**:
- External third-party services only (payment gateways, external APIs)
- Nothing internal to the application

**Key difference from integration tests**:
- Integration: "Does DockerEnvironment work correctly with Docker?"
- E2E: "Can a user create a task that runs in Docker and see results?"

## Test Naming and Performance Requirements

### File Naming Conventions
- Unit tests: `*.test.ts` or `*.unit.test.ts` 
- Integration tests: `*.integration.test.ts`
- E2E tests: `*.e2e.test.ts`

## Data Directory Management

### Test Setup
All backend tests automatically get isolated temporary directories:

```typescript
// This is handled automatically by test setup
// In your tests, just use:
import { getDataDir } from '../utils/data-dir.js'

const dataDir = getDataDir() // Returns temp directory in tests
```

### Production vs Test Behavior
- **Production**: `getDataDir()` returns `~/.moatless` or `DATA_DIR` env var
- **Tests**: `getDataDir()` returns isolated temp directory
- **Never**: Access `process.env.DATA_DIR` directly

## Common Testing Patterns

### Pattern 1: Service with External Dependencies

```typescript
// ALWAYS write unit tests (mandatory)
describe('TaskService - Unit Tests', () => {
  let mockStorage: MockedStorage
  let mockWorkspace: MockedWorkspace
  let service: TaskService

  beforeEach(() => {
    mockStorage = createMockStorage()
    mockWorkspace = createMockWorkspace()
    service = new TaskService(mockStorage, mockWorkspace)
  })

  it('should validate task parameters', () => {
    // Test pure logic with mocks
  })

  it('should handle storage errors gracefully', () => {
    // Test error handling
  })

  it('should emit correct events', () => {
    // Test event emission logic
  })
})

// OPTIONALLY add integration tests if needed
describe('TaskService - Integration Tests', () => {
  let service: TaskService
  let tempDir: string

  beforeEach(async () => {
    tempDir = await createTempDir()
    const storage = new FileStorage(tempDir)
    const workspace = new WorkspaceService(storage)
    service = new TaskService(storage, workspace)
  })

  it('should persist task to storage', async () => {
    // Test with real file system
  })

  it('should handle concurrent task creation', async () => {
    // Test real concurrency scenarios
  })
})
```

### Pattern 2: External System Integration

```typescript
// For components that integrate with external systems (Docker, Git, etc.)

// ALWAYS write unit tests (mandatory)
describe('DockerEnvironment - Unit Tests', () => {
  beforeEach(() => {
    vi.mock('node:child_process', () => ({
      execSync: vi.fn((cmd) => {
        if (cmd.includes('docker inspect')) return 'running'
        if (cmd.includes('docker create')) return 'container-id'
        return 'mocked'
      })
    }))
  })

  it('should handle state transitions correctly', () => {
    // Test business logic with mocked Docker
  })

  it('should recover from error states', () => {
    // Test error recovery logic
  })

  it('should clean up resources on teardown', () => {
    // Test cleanup logic
  })
})

// OPTIONALLY add integration tests to verify Docker integration
describe('DockerEnvironment - Integration Tests', () => {
  // This tests ONLY DockerEnvironment + Docker, not the full app
  let env: DockerEnvironment

  beforeEach(() => {
    // Create DockerEnvironment instance directly
    env = new DockerEnvironment('test-task', mockConfig, '/test/workspace')
  })

  afterEach(async () => {
    // Clean up real Docker resources
    await env.teardown()
  })

  it('should create real Docker container', async () => {
    await env.setup()
    
    // Verify real container exists
    const result = await execAsync('docker ps')
    expect(result).toContain(env.getContainerName())
  })

  it('should execute commands in real container', async () => {
    await env.setup()
    const result = await env.runCommand('echo "Hello from Docker"')
    expect(result.exitCode).toBe(0)
    expect(result.output).toBe('Hello from Docker')
  })

  it('should recover from container in error state', async () => {
    // Test the self-healing behavior with real Docker
    await env.setup()
    
    // Force container into error state
    await execAsync(`docker kill ${env.getContainerName()}`)
    
    // Should recover
    await env.ensureContainerReady()
    const result = await env.runCommand('echo "Recovered"')
    expect(result.exitCode).toBe(0)
  })
})
```

### Pattern 3: API Routes

```typescript
// For API route testing

// ALWAYS write unit tests for route handlers
describe('Task Routes - Unit Tests', () => {
  let mockTaskService: MockedTaskService

  beforeEach(() => {
    mockTaskService = createMockTaskService()
  })

  it('should validate request payload', async () => {
    // Test validation logic
  })

  it('should handle service errors', async () => {
    // Test error handling
  })
})

// OFTEN write integration tests for API endpoints
describe('Task Routes - Integration Tests', () => {
  let app: App
  let mockTaskService: MockedTaskService

  beforeEach(() => {
    mockTaskService = createMockTaskService()
    app = buildApp({ taskService: mockTaskService })
  })

  it('should handle POST /tasks', async () => {
    const res = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: { ... }
    })
    expect(res.statusCode).toBe(201)
  })
})

// OPTIONALLY add E2E tests for critical workflows
describe('Task Routes - E2E Tests', () => {
  let server: Server

  beforeAll(async () => {
    server = await startRealServer()
  })

  it('should create and execute task', async () => {
    // Test complete flow with real services
  })
})
```

## Anti-Patterns to Avoid

### 1. Missing Unit Tests
**WRONG**: Writing only integration or E2E tests without unit tests
```typescript
// DON'T DO THIS
describe('DockerEnvironment - Integration Tests', () => { ... })
// Missing unit tests!
```

**RIGHT**: Always have unit tests, add other levels as needed
```typescript
// DO THIS - Always have unit tests
describe('DockerEnvironment - Unit Tests', () => {
  // Test all business logic with mocked Docker
})

// Optionally add integration tests if needed
describe('DockerEnvironment - Integration Tests', () => {
  // Test real Docker container creation
})
```

### 2. Fake Integration Tests
**WRONG**: "Integration" tests that expect failures due to missing dependencies
```typescript
// DON'T DO THIS
describe('DockerEnvironment - Integration Tests', () => {
  it('should fail when container does not exist', async () => {
    // This is not integration testing!
    const result = await env.runCommand('echo test')
    expect(result.error).toMatch(/container not found/)
  })
})
```

**RIGHT**: Integration tests should test successful integration
```typescript
// DO THIS
describe('DockerEnvironment - Integration Tests', () => {
  it('should execute commands in real container', async () => {
    await env.setup() // Creates real container
    const result = await env.runCommand('echo test')
    expect(result.output).toBe('test')
  })
})
```

### 3. Over-Mocking in Integration Tests
**WRONG**: Mocking everything in an integration test
```typescript
// DON'T DO THIS in integration tests
describe('TaskService - Integration Tests', () => {
  let mockStorage, mockWorkspace, mockEventBus // Too much mocking!
})
```

**RIGHT**: Use real implementations in integration tests
```typescript
// DO THIS
describe('TaskService - Integration Tests', () => {
  let realStorage, realWorkspace // Use real implementations
})
```

## Test Selection Examples

### Example: Docker-based Task Execution

This example shows how the same feature (Docker task execution) is tested at different levels:

**Unit Test** (test business logic):
```typescript
describe('DockerEnvironment - Unit Tests', () => {
  it('should transition to error state on container failure', () => {
    // Mock Docker commands
    vi.mocked(execSync).mockImplementation((cmd) => {
      if (cmd.includes('docker inspect')) throw new Error('Container crashed')
    })
    
    const env = new DockerEnvironment(...)
    expect(env.getStatus()).toBe('error')
  })
})
```

**Integration Test** (test component + Docker):
```typescript
describe('DockerEnvironment - Integration Tests', () => {
  it('should manage real Docker container lifecycle', async () => {
    const env = new DockerEnvironment('task-1', config, '/workspace')
    
    // Test with real Docker
    await env.setup()
    const result = await env.runCommand('npm install')
    expect(result.exitCode).toBe(0)
    
    await env.teardown()
    // Verify container is removed
  })
})
```

**E2E Test** (test complete workflow):
```typescript
describe('Docker Task Execution - E2E Tests', () => {
  it('should execute task in Docker from API to completion', async () => {
    // Create repository with Docker config
    const repo = await api.post('/repositories', {
      name: 'Test Repo',
      codeExecution: { type: 'docker', image: 'node:18' }
    })
    
    // Create and start task
    const task = await api.post('/tasks', {
      repositoryId: repo.id,
      message: 'Create index.js with console.log("Hello")'
    })
    
    // Wait for completion
    await waitForTaskCompletion(task.id)
    
    // Verify file was created
    const file = await api.get(`/tasks/${task.id}/files/index.js`)
    expect(file.content).toContain('console.log("Hello")')
  })
})
```

### Example 1: Testing a Caching Service

**Scenario**: Service that caches API responses in Redis

**Unit Test** (test caching logic):
```typescript
it('should return cached value when available', () => {
  const mockRedis = { get: vi.fn().mockResolvedValue('cached') }
  const service = new CacheService(mockRedis)
  // Test caching algorithm
})
```

**Integration Test** (test with real Redis):
```typescript
it('should store and retrieve from Redis', async () => {
  const redis = new Redis({ host: 'localhost' })
  const service = new CacheService(redis)
  // Test actual Redis operations
})
```

### Example 2: Testing File Processing

**Scenario**: Service that processes uploaded files

**Unit Test** (test processing logic):
```typescript
it('should validate file format', () => {
  const mockFile = { name: 'test.pdf', size: 1000 }
  const result = service.validateFile(mockFile)
  // Test validation logic
})
```

**Integration Test** (test with real files):
```typescript
it('should process uploaded file', async () => {
  const tempFile = await createTempFile('test.pdf')
  const result = await service.processFile(tempFile)
  // Test actual file operations
})
```

### Example 3: Testing Authentication Flow

**Scenario**: Complete authentication system

**E2E Test** (test complete flow):
```typescript
it('should handle login → authenticate → access protected resource', async () => {
  // Create user
  await api.post('/register', { email, password })
  
  // Login
  const { token } = await api.post('/login', { email, password })
  
  // Access protected resource
  const result = await api.get('/protected', {
    headers: { Authorization: `Bearer ${token}` }
  })
  
  expect(result.status).toBe(200)
})
```

## Quick Decision Guide

Follow this process:

1. **Always start with unit tests** → Test all business logic, error handling, state management
2. **Do I need to verify this component works with a real external system?** → Add integration tests
3. **Do I need to verify a complete user workflow through multiple components?** → Add E2E tests

### Scope-Based Examples:

**DockerEnvironment**:
- Unit tests: Mock Docker, test state management, error handling
- Integration tests: Real Docker, test container lifecycle (just DockerEnvironment + Docker)
- E2E tests: Full workflow (API → Task → DockerEnvironment → Agent → Results)

**Git Service**:
- Unit tests: Mock git commands, test branch logic
- Integration tests: Real git repos, test clone/commit/push (just GitService + Git)
- E2E tests: Create repo → make changes → view diff in UI

**API Routes**:
- Unit tests: Test validation, error handling logic
- Integration tests: Test route with mocked services (just route + HTTP)
- E2E tests: Test complete user actions through API

### Key Question:
- "Am I testing how ONE component uses an external system?" → Integration test
- "Am I testing how MULTIPLE components work together for a user?" → E2E test

## Special Cases

### Server/Process Management Tests
- Always use E2E tests (*.e2e.test.ts)
- These inherently take >2s due to process startup

### SSE/WebSocket Tests
- Unit tests: Mock connections
- E2E tests: Use real connections

### Database Tests
- Unit tests: Mock database calls
- Integration tests: Use test database

### External API Tests
- Unit tests: Mock HTTP calls
- Integration tests: Use API mocks/stubs
- E2E tests: Consider using sandbox environments

## Remember

1. **Unit tests are MANDATORY** for all components - test business logic with mocks
2. **Integration tests** = One component + its external system (e.g., DockerEnvironment + Docker)
3. **E2E tests** = Complete user workflows through multiple components
4. **Choose by SCOPE, not performance** - Integration tests can be slow if needed
5. **Name files correctly** to indicate test level
6. **Mock appropriately**:
   - Unit: Mock everything external
   - Integration: Mock other app components, use real external system
   - E2E: Mock only third-party services

Test priority:
- Always: Comprehensive unit tests
- Often: Integration tests for components using external systems
- Selectively: E2E tests for critical user journeys

The key distinction:
- Integration tests verify "Does this component correctly use Docker/Git/Database?"
- E2E tests verify "Can a user complete their task using our system?"