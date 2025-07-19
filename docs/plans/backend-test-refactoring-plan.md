# Backend Test Refactoring Action Plan

## Overview

This document outlines a comprehensive plan to refactor the backend test suite to:
1. Align with the updated testing guidelines (unit + integration tests only)
2. Implement shared mock factories and test utilities
3. Ensure consistency and maintainability across all tests
4. Remove E2E terminology in favor of broad integration tests

## Current State Analysis

### Issues to Address
- E2E test terminology still present in file names and descriptions
- Duplicate mock implementations across test files
- Inconsistent test data creation
- Missing shared test utilities
- Lack of centralized mock factories

### Existing Test Files Requiring Updates
- Files with `.e2e.test.ts` suffix need renaming to `.integration.test.ts`
- Test descriptions mentioning "E2E" need updating
- Inline mocks need extraction to shared modules

## Target Architecture

### Shared Test Infrastructure (in @moatless/types)
```
packages/types/
├── src/
│   ├── test-utils/              # Shared test infrastructure
│   │   ├── index.ts            # Barrel export
│   │   ├── mocks/              # Mock factories (frontend + backend)
│   │   │   ├── index.ts        # Barrel export
│   │   │   ├── task.mock.ts    # Task-related mocks
│   │   │   ├── repository.mock.ts # Repository mocks
│   │   │   ├── message.mock.ts # Message mocks
│   │   │   ├── event.mock.ts   # Event mocks
│   │   │   └── workspace.mock.ts # Workspace mocks
│   │   └── factories/          # Test data builders
│   │       ├── index.ts        # Barrel export
│   │       ├── task.factory.ts # TaskBuilder class
│   │       ├── repository.factory.ts # RepositoryBuilder
│   │       └── event.factory.ts # EventStreamBuilder
│   └── index.ts                # Update to export test-utils
```

### Backend-Specific Test Infrastructure
```
apps/backend/
├── test/
│   ├── setup.ts                # Global test setup (exists)
│   ├── mocks/                  # Backend-specific mocks
│   │   ├── index.ts           # Barrel export
│   │   ├── environment.mock.ts # Environment mocks (backend-specific)
│   │   ├── services.mock.ts    # Service mocks (backend-specific)
│   │   └── git-utils.mock.ts  # Git utils mocks (exists)
│   └── helpers/                # Backend-specific test utilities
│       ├── index.ts           # Barrel export
│       ├── test-utils.ts      # Backend test utilities
│       ├── docker.helper.ts   # Docker test helpers
│       └── git.helper.ts      # Git test helpers
```

## Phase 1: Infrastructure Setup

### Task 1.1: Create Mock Factory Structure
**Priority**: High  
**Files to create**:
- `/packages/types/src/test-utils/index.ts`
- `/packages/types/src/test-utils/mocks/index.ts`
- `/packages/types/src/test-utils/factories/index.ts`
- `/apps/backend/test/mocks/index.ts`
- `/apps/backend/test/helpers/index.ts`

**Actions**:
1. Create directory structure in @moatless/types package
2. Set up barrel exports for easy imports
3. Update package.json exports in @moatless/types:
   ```json
   {
     "exports": {
       ".": "./dist/index.js",
       "./test-utils": "./dist/test-utils/index.js"
     }
   }
   ```
4. Configure TypeScript paths for both frontend and backend

### Task 1.2: Implement Core Mock Factories
**Priority**: High  
**Dependencies**: Task 1.1

#### Task 1.2.1: Create task.mock.ts in @moatless/types
**File**: `/packages/types/src/test-utils/mocks/task.mock.ts`
```typescript
import type { Task, TaskIteration, GitDiffFile, GitDiffResponse } from '../../types'

// Example structure - no vitest dependency, pure functions
export function createMockTask(overrides?: Partial<Task>): Task {
  return {
    taskId: 'test-task-123',
    repositoryId: 'test-repo',
    status: 'open',
    createdAt: new Date(),
    // ... other defaults
    ...overrides
  }
}

export function createMockTaskIteration(overrides?: Partial<TaskIteration>): TaskIteration
export function createMockTaskWithIterations(iterationCount?: number): Task
export function createMockGitDiffFile(overrides?: Partial<GitDiffFile>): GitDiffFile
export function createMockGitDiffResponse(fileCount?: number): GitDiffResponse
```

#### Task 1.2.2: Create repository.mock.ts in @moatless/types
**File**: `/packages/types/src/test-utils/mocks/repository.mock.ts`
```typescript
import type { Repository } from '../../types'

export function createMockRepository(overrides?: Partial<Repository>): Repository
export function createMockDockerRepository(overrides?: Partial<Repository>): Repository
export function createMockRepositoryWithPreview(overrides?: Partial<Repository>): Repository
```

**Note**: These mock factories should be pure functions without any testing framework dependencies, making them usable in any test environment (Vitest, Jest, etc.)

#### Task 1.2.3: Create message.mock.ts
```typescript
export function createMockMessage(overrides?: Partial<Message>): Message
export function createMockUserMessage(content: string, overrides?: Partial<Message>): Message
export function createMockAssistantMessage(content: string, overrides?: Partial<Message>): Message
export function createMockMessageWithToolUse(overrides?: Partial<Message>): Message
```

#### Task 1.2.4: Create event.mock.ts
```typescript
export function createMockEvent(overrides?: Partial<Event>): Event
export function createMockMessageEvent(overrides?: Partial<Event>): Event
export function createMockErrorEvent(error: string, overrides?: Partial<Event>): Event
export function createMockWorkspaceEvent(overrides?: Partial<Event>): Event
export function createMockTaskStatusEvent(status: string, overrides?: Partial<Event>): Event
```

#### Task 1.2.5: Create environment.mock.ts
```typescript
export function createMockLocalEnvironment(overrides?: Partial<BaseEnvironment>): BaseEnvironment
export function createMockDockerEnvironment(overrides?: Partial<BaseEnvironment>): BaseEnvironment
export function createMockEnvironmentWithError(error: string): BaseEnvironment
```

#### Task 1.2.6: Create services.mock.ts in backend test directory
**File**: `/apps/backend/test/mocks/services.mock.ts`
```typescript
import { vi } from 'vitest'
import type { StorageBackend } from '../../services/storage'

// Backend-specific mocks that depend on vitest
export function createMockStorageBackend(overrides?: Partial<StorageBackend>): StorageBackend {
  return {
    exists: vi.fn().mockResolvedValue(true),
    read: vi.fn().mockResolvedValue({}),
    write: vi.fn().mockResolvedValue(undefined),
    delete: vi.fn().mockResolvedValue(undefined),
    list: vi.fn().mockResolvedValue([]),
    ...overrides
  }
}

export function createMockLogger(): Logger
export function createMockSettingsService(overrides?: any): SettingsService
export function createMockEventService(): EventService
```

**Note**: Service mocks remain in backend since they depend on vitest and backend-specific interfaces

### Task 1.3: Implement Test Data Builders
**Priority**: Medium  
**Dependencies**: Task 1.2

#### Task 1.3.1: Create task.factory.ts
```typescript
export class TaskBuilder {
  withId(id: string): this
  withStatus(status: TaskStatus): this
  withIterations(count: number): this
  withRepository(repoId: string): this
  withGitDiff(diff: GitDiffResponse): this
  build(): Task
}
```

#### Task 1.3.2: Create repository.factory.ts
```typescript
export class RepositoryBuilder {
  withId(id: string): this
  withName(name: string): this
  withDockerConfig(config: DockerConfig): this
  withPreviewConfig(config: PreviewConfig): this
  build(): Repository
}
```

#### Task 1.3.3: Create event.factory.ts
```typescript
export class EventStreamBuilder {
  addMessageEvent(message: string): this
  addErrorEvent(error: string): this
  addStatusEvent(status: string): this
  addDelay(ms: number): this
  build(): AsyncGenerator<Event>
}
```

### Task 1.4: Implement Test Helpers
**Priority**: Medium  
**Dependencies**: Task 1.1

#### Task 1.4.1: Create test-utils.ts
```typescript
export async function createTempDir(prefix?: string): Promise<string>
export async function waitFor(condition: () => boolean, timeout?: number): Promise<void>
export function resetAllMocks(): void
export async function setupTestEnvironment(): Promise<void>
```

#### Task 1.4.2: Create docker.helper.ts
```typescript
export async function ensureDockerAvailable(): Promise<boolean>
export async function cleanupDockerContainers(prefix: string): Promise<void>
export async function waitForContainerReady(containerName: string): Promise<void>
export function createDockerTestConfig(): DockerConfig
```

#### Task 1.4.3: Create git.helper.ts
```typescript
export async function createTestGitRepo(dir: string): Promise<void>
export async function createTestBranch(dir: string, branchName: string): Promise<void>
export async function addTestCommit(dir: string, message: string): Promise<void>
export function createGitDiffFixture(fileCount: number): GitDiffResponse
```

## Phase 2: Test Migration (E2E → Integration)

### Task 2.1: Rename E2E Test Files
**Priority**: High  
**Pattern**: `*.e2e.test.ts` → `*.integration.test.ts`

**Files to rename**:
1. Find all files: `find apps/backend -name "*.e2e.test.ts"`
2. Rename each file
3. Update imports in other files if necessary

### Task 2.2: Update Test Descriptions
**Priority**: High  
**Dependencies**: Task 2.1

**Actions**:
1. Search for "E2E" in test descriptions
2. Replace with appropriate terminology:
   - "E2E test" → "Integration test (broad)"
   - "end-to-end" → "complete workflow"
   - Update comments to reflect new terminology

### Task 2.3: Update vitest.config.ts
**Priority**: Medium  
**File**: `/apps/backend/vitest.config.ts`

**Changes**:
- Remove `'**/*.e2e.test.{ts,js}'` from include patterns
- Update exclude patterns if necessary
- Remove any E2E-specific configuration

## Phase 3: Mock Consolidation

### Task 3.1: Identify Duplicate Mocks
**Priority**: High

**Actions**:
1. Grep for `vi.mock` patterns across all test files
2. Create inventory of commonly mocked modules
3. Prioritize mocks that appear in 3+ files

### Task 3.2: Replace Inline Mocks
**Priority**: Medium  
**Dependencies**: Phase 1 complete

**For each test file**:
1. Import shared mocks from @moatless/types:
   ```typescript
   // For data mocks (no test framework dependency)
   import { createMockTask, createMockRepository } from '@moatless/types/test-utils'
   
   // For backend-specific mocks (with vi.fn())
   import { createMockStorageBackend } from '@test/mocks'
   ```
2. Replace inline mock creation with factory calls
3. Remove duplicate mock implementations
4. Update test assertions if necessary

### Task 3.3: Standardize Mock Patterns
**Priority**: Medium

**Patterns to enforce**:
- Use `vi.mocked()` for type safety
- Consistent mock reset in `afterEach`
- Proper mock cleanup in integration tests
- Smart mocks for stateful components

## Phase 4: Documentation Updates

### Task 4.1: Update backend-testing.md
**Priority**: High  
**File**: `/docs/patterns/backend-testing.md`

**Add sections**:
- "Test Infrastructure" describing mock structure
- "Using Shared Mocks" with examples
- "Test Data Factories" usage guide
- "Mock Reference" listing all available mocks

### Task 4.2: Create Mock Usage Guide
**Priority**: Medium  
**File**: `/docs/patterns/backend-mock-usage.md`

**Contents**:
- How to use each mock factory
- Common patterns and combinations
- Best practices for extending mocks
- Migration guide from inline to shared mocks

### Task 4.3: Update Contributing Guidelines
**Priority**: Low  
**Files**: Contributing docs if they exist

**Updates**:
- Reference new test structure
- Mandate use of shared mocks
- Include test writing guidelines

## Progress Update (2024-01-19)

### ✅ COMPLETED High Priority Tasks
1. [x] **Task 1.1: Create mock factory structure** ✅
   - Created `/packages/types/src/test-utils/index.ts`
   - Created `/packages/types/src/test-utils/mocks/index.ts`
   - Created `/packages/types/src/test-utils/factories/index.ts`
   - Created `/apps/backend/test/mocks/index.ts`
   - Created `/apps/backend/test/helpers/index.ts`
   - Updated `packages/types/package.json` with test-utils export

2. [x] **Task 1.2: Implement core mock factories** ✅
   - **Task 1.2.1**: Created `/packages/types/src/test-utils/mocks/task.mock.ts` with:
     - `createMockTask()`, `createMockTaskIteration()`, `createMockTaskWithIterations()`
     - `createMockGitDiffFile()`, `createMockGitDiffResponse()`, `createMockGitStatus()`
   - **Task 1.2.2**: Created `/packages/types/src/test-utils/mocks/repository.mock.ts` with:
     - `createMockRepository()`, `createMockCodeExecution()`, `createMockDockerCodeExecution()`
     - `createMockPreviewConfig()`, `createMockRepositoryWithPreview()`, `createMockDockerRepository()`
     - `createMockRepositoryList()`, `createMockBranchListResponse()`, `createMockFileSearchResponse()`
   - **Task 1.2.3**: Created `/packages/types/src/test-utils/mocks/message.mock.ts` with:
     - `createMockMessage()`, `createMockUserMessage()`, `createMockAssistantMessage()`
     - `createMockTextContent()`, `createMockToolUseContent()`, `createMockToolResult()`
     - `createMockMessageWithToolUse()`, `createMockMessageWithToolResult()`, `createMockMessagesResponse()`
     - `createMockToolCallGroup()`
   - **Task 1.2.4**: Created `/packages/types/src/test-utils/mocks/event.mock.ts` with:
     - `createMockEvent()`, `createMockMessageEvent()`, `createMockErrorEvent()`
     - `createMockWorkspaceEvent()`, `createMockServerEvent()`, `createMockResultEvent()`
     - `createMockTaskStatusEvent()`, `createMockEventListResponse()`, `createMockStreamStatusResponse()`
     - `createMockEventStream()` for testing SSE streams
   - **Task 1.2.5**: Created `/packages/types/src/test-utils/mocks/environment.mock.ts` with:
     - `createMockLocalWorkspace()`, `createMockDockerWorkspace()`, `createMockWorkspace()`
     - `createMockWorkspaceWithError()`, `createMockWorkspaceCapabilities()`, `createMockWorkspaceStatusResponse()`
     - `createMockFileTreeWithChanges()`, `createMockSyncWorkspaceResponse()`, `createMockSyncWorkspaceWithConflicts()`
     - Preview environment mocks: `createMockPreviewEnvironmentMetadata()`, `createMockProcessValidation()`
   - **Task 1.2.6**: Created `/apps/backend/test/mocks/services.mock.ts` with:
     - `createMockStorageBackend()`, `createMockStorageWithData()`, `createMockEventService()`
     - `createMockLogger()`, `createMockSettingsService()`, `createMockAgentService()`
     - `createMockTaskService()`, `createMockWorkspaceService()`

3. [x] **Task 2.1: Rename E2E test files** ✅
   - Renamed 10 test files from `.e2e.test.ts` to `.integration.test.ts`:
     - `complete-workflow.e2e.test.ts` → `complete-workflow.integration.test.ts`
     - `docker-workspace-e2e.test.ts` → `docker-workspace.integration.test.ts`
     - `graceful-shutdown.e2e.test.ts` → `graceful-shutdown.integration.test.ts`
     - `settings-workflow.e2e.test.ts` → `settings-workflow.integration.test.ts`
     - `server-lifecycle-e2e.test.ts` → `server-lifecycle.integration.test.ts`
     - `preview-environment-proxy.e2e.test.ts` → `preview-environment-proxy-broad.integration.test.ts`
     - `repositories.e2e.test.ts` → `repositories-broad.integration.test.ts`
     - `repositories-suggestions.e2e.test.ts` → `repositories-suggestions-broad.integration.test.ts`
     - `events-sse-real-server.e2e.test.ts` → `events-sse-real-server.integration.test.ts`
     - `preview-environment-shutdown.e2e.test.ts` → `preview-environment-shutdown.integration.test.ts`

4. [x] **Task 2.2: Update test descriptions** ✅
   - Updated all `describe()` blocks to remove "E2E" terminology
   - Replaced with "Integration Test (broad)" or "Integration Tests (broad)"
   - Updated comments in 20+ test files to use proper terminology

5. [x] **Task 3.1: Identify duplicate mocks** ✅
   - **Logger mocks**: Found 11 duplicate `vi.mock('../utils/logger.js')` instances
   - **Port allocation mocks**: Found 4 duplicate `vi.mock('../utils/port-allocation.js')` instances  
   - **Node.js fs mocks**: Found 4 duplicate `vi.mock('node:fs')` instances
   - **Settings service mocks**: Multiple duplicate implementations
   - **Storage backend mocks**: Various inline implementations across tests

6. [x] **Task 2.3: Update vitest.config.ts** ✅
   - Removed `'**/*.e2e.test.{ts,js}'` from include patterns in `/apps/backend/vitest.config.ts`
   - Configuration now only includes unit and integration test patterns

### 🔄 IN PROGRESS Medium Priority Tasks
7. [ ] **Task 1.3: Implement test data builders** - Ready to start
8. [ ] **Task 1.4: Implement test helpers** - Ready to start  
9. [ ] **Task 3.2: Replace inline mocks** - **NEXT PRIORITY**
10. [ ] **Task 3.3: Standardize mock patterns** - Depends on 3.2
11. [ ] **Task 4.2: Create mock usage guide** - Pending
12. [ ] **Task 4.1: Update backend-testing.md** - Pending

### ⏳ Remaining Low Priority Tasks
13. [ ] **Task 4.3: Update contributing guidelines** - Final task

## 🎯 Next Steps for LLM

The **highest priority** remaining task is **Task 3.2: Replace inline mocks with shared factories**.

### **Files Created and Ready to Use:**

**Shared Mock Factories (Framework Agnostic):**
- `/packages/types/src/test-utils/mocks/task.mock.ts` - Task, GitDiff, GitStatus mocks
- `/packages/types/src/test-utils/mocks/repository.mock.ts` - Repository, CodeExecution, PreviewConfig mocks  
- `/packages/types/src/test-utils/mocks/message.mock.ts` - Message, ToolResult, ToolCallGroup mocks
- `/packages/types/src/test-utils/mocks/event.mock.ts` - Event, MessageEvent, ErrorEvent mocks
- `/packages/types/src/test-utils/mocks/environment.mock.ts` - Workspace, PreviewEnvironment mocks

**Backend Service Mocks (Vitest-dependent):**
- `/apps/backend/test/mocks/services.mock.ts` - StorageBackend, EventService, Logger mocks

**Barrel Exports for Easy Importing:**
- `/packages/types/src/test-utils/index.ts` - Main export
- `/packages/types/src/test-utils/mocks/index.ts` - All mock factories
- `/apps/backend/test/mocks/index.ts` - Backend service mocks

### **Target Files for Mock Replacement:**

**High-Priority Files with Logger Mocks (11 instances):**
1. `/apps/backend/environments/__tests__/local-environment.unit.test.ts`
2. `/apps/backend/environments/__tests__/docker-environment.unit.test.ts` 
3. `/apps/backend/agents/__tests__/claude-code-agent-settings.unit.test.ts`
4. `/apps/backend/__tests__/settings-workflow.integration.test.ts`
5. `/apps/backend/routes/__tests__/settings.integration.test.ts`
6. `/apps/backend/services/__tests__/preview-environment.service.integration.test.ts`
7. `/apps/backend/services/__tests__/settings.service.unit.test.ts`
8. `/apps/backend/services/__tests__/workspace.service.integration.test.ts`
9. `/apps/backend/services/__tests__/agent.service.unit.test.ts`
10. `/apps/backend/services/__tests__/preview-server.unit.test.ts`
11. `/apps/backend/utils/__tests__/logger.unit.test.ts`

**Files with Port Allocation Mocks (4 instances):**
1. `/apps/backend/environments/__tests__/docker-environment.integration.test.ts`
2. `/apps/backend/environments/__tests__/docker-environment.unit.test.ts`
3. `/apps/backend/services/__tests__/workspace.unit.test.ts`
4. `/apps/backend/services/__tests__/preview-server.unit.test.ts`

**Files with Node.js fs Mocks (4 instances):**
1. `/apps/backend/environments/__tests__/local-environment.unit.test.ts`
2. `/apps/backend/services/__tests__/preview-environment.service.integration.test.ts`
3. `/apps/backend/services/__tests__/workspace.unit.test.ts`
4. `/apps/backend/services/__tests__/file-operations.service.test.ts`

### **Example Migration Pattern:**

**Before:**
```typescript
vi.mock('../../utils/logger.js', () => ({
  logger: {
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    debug: vi.fn(),
  },
  createTaskLogger: vi.fn().mockReturnValue({
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  })
}))
```

**After:**
```typescript
import { createMockLogger } from '../../test/mocks/services.mock.js'

vi.mock('../../utils/logger.js', () => ({
  logger: createMockLogger(),
  createTaskLogger: vi.fn().mockReturnValue(createMockLogger())
}))
```

### **How to Use the Created Mock Factories:**

**Import patterns:**
```typescript
// For pure data mocks (no test framework dependency)
import { 
  createMockTask, 
  createMockRepository, 
  createMockEvent 
} from '@moatless/types/test-utils'

// For backend service mocks (with vi.fn())
import { 
  createMockStorageBackend, 
  createMockLogger,
  createMockEventService 
} from '../test/mocks/services.mock.js'
```

**Usage in tests:**
```typescript
const mockTask = createMockTask({ 
  taskId: 'custom-id', 
  status: 'completed' 
})

const mockStorage = createMockStorageBackend({
  getTask: vi.fn().mockResolvedValue(mockTask)
})
```

## 🧹 **Phase 5: Test Cleanup and Consolidation**

### **Files/Tests Identified for Removal**

#### **Obsolete Skipped Tests** 
These tests are skipped with clear removal indicators and should be deleted:

1. **`complete-workflow.integration.test.ts:491`**
   ```typescript
   it.skip('should track agent status changes during execution - REMOVED: agent status no longer part of API', async () => {
   ```
   - **Reason**: Agent status functionality removed from API
   - **Action**: Delete this entire test block (lines 491-580+)

2. **`worker-api.integration.test.ts:226`**
   ```typescript
   it.skip('should start worker mode and fetch task/repository from API - SKIPPED: Worker workspace initialization needs refactoring', async () => {
   ```
   - **Reason**: Worker workspace initialization needs refactoring
   - **Action**: Delete or fix the test, don't leave it skipped indefinitely

#### **Redundant Mock Files**
After implementing shared mock factories, these files have overlapping functionality:

1. **`/apps/backend/test/mocks/workspace.mock.ts`**
   - **Overlaps with**: New environment.mock.ts and services.mock.ts
   - **Contains**: Agent, Environment, and Workspace mocks
   - **Action**: Consolidate functions into shared mocks, then delete
   - **Migration**: 
     - Move `createMockAgent()` → services.mock.ts
     - Move workspace functions → environment.mock.ts  
     - Update imports in dependent test files

2. **`/apps/backend/test/mocks/git-utils.mock.ts`**
   - **Status**: Keep - specialized git utility mocks
   - **Action**: No removal needed, but could benefit from consolidation

#### **Skipped Tests Requiring Decision**

**SSE/Events Tests** (8 skipped tests in events-global-stream.integration.test.ts):
```typescript
it.skip('should establish SSE connection and set proper headers', async () => {
it.skip('should register global connection with SSE manager', async () => {
// ... 6 more skipped SSE tests
```
- **Reason**: Unclear why skipped - may be infrastructure related  
- **Action**: **Investigate and either fix or remove** - don't leave permanently skipped
- **Priority**: Medium - affects real-time functionality

**Rate Limiting Test** (error-handling.integration.test.ts:114):
```typescript
it.skip('should return 429 when rate limit is exceeded', async () => {
```
- **Reason**: Rate limiting not implemented or needs refactoring
- **Action**: **Remove if rate limiting not planned**, or implement properly

#### **Workspace Setup Tests** (2 skipped tests):
```typescript
// workspace.unit.test.ts:252
it.skip('should handle workspace setup failures', async () => {

// agent-workspace-recreation tests (2 files)
it.skip('should throw error if workspace recreation fails', async () => {
```
- **Reason**: Workspace failure handling needs implementation
- **Action**: **Implement proper error handling** or remove if not needed

### **Task 5.1: Remove Obsolete Tests**
**Priority**: Medium  
**Files to Update**:

1. **Remove confirmed obsolete tests**:
   - Delete agent status test in complete-workflow.integration.test.ts
   - Remove or fix worker initialization test

2. **Investigate skipped tests**:
   - SSE tests: Fix infrastructure or remove if not needed
   - Rate limiting: Remove if not implementing feature
   - Workspace error handling: Implement or remove

### **Task 5.2: Consolidate Mock Files**
**Priority**: Medium  
**Dependencies**: Task 3.2 (Replace inline mocks)

1. **Migrate workspace.mock.ts functions**:
   ```typescript
   // Move to services.mock.ts
   export { createMockAgent } from '../test/mocks/services.mock.js'
   
   // Move to environment.mock.ts  
   export { createMockWorkspace, createMockDockerWorkspace } from '@moatless/types/test-utils'
   ```

2. **Update dependent files** (7+ files import workspace.mock.ts):
   ```bash
   # Find files importing workspace.mock.ts
   grep -r "workspace\.mock" apps/backend --include="*.ts"
   ```

3. **Delete workspace.mock.ts** after migration complete

### **Task 5.3: Standardize Skip Patterns**
**Priority**: Low

**Current skip patterns found**:
- `it.skip('description - REMOVED: reason')` ✅ Good - clear removal reason
- `it.skip('description - SKIPPED: reason')` ❌ Unclear - needs decision  
- `it.skip('description')` ❌ Bad - no reason given

**Standard pattern to enforce**:
```typescript
// For tests to be removed
it.skip('test name - REMOVE: specific reason why obsolete', async () => {

// For tests temporarily disabled (should be rare)
it.skip('test name - TODO: what needs to be implemented/fixed', async () => {
```

### **Benefits of Cleanup**:
1. **Reduced maintenance burden** - no dead code to maintain
2. **Clearer test intent** - only valid, working tests remain  
3. **Improved CI speed** - fewer skipped tests to process
4. **Better developer experience** - no confusion about test status
5. **Consolidated mocking** - single source of truth for test data

### **Cleanup Success Criteria**:
- [ ] Zero skipped tests without clear removal/fix timeline
- [ ] No duplicate mock implementations
- [ ] All test files have clear purpose and work correctly
- [ ] No `.skip()` tests older than 30 days without resolution plan

## Migration Checklist

### Per Test File Checklist
- [ ] File renamed if needed (.e2e.test.ts → .integration.test.ts)
- [ ] Test descriptions updated (no "E2E" terminology)
- [ ] Inline mocks replaced with shared factories
- [ ] Import statements updated
- [ ] Proper cleanup implemented
- [ ] Tests still pass

### Global Checklist
- [ ] All mock factories implemented
- [ ] All test helpers created
- [ ] Documentation updated
- [ ] No .e2e.test.ts files remain
- [ ] CI/CD configuration updated
- [ ] Team notified of changes

## Success Criteria

The refactoring is complete when:
1. No files with `.e2e.test.ts` extension exist
2. All tests use shared mock factories from @moatless/types where possible
3. No duplicate mock implementations exist across frontend and backend
4. Documentation reflects new structure
5. All tests pass in both frontend and backend
6. Test execution time improved (due to better mocking)
7. New tests follow the established patterns
8. Frontend and backend tests use the same mock data structures
9. @moatless/types package exports test utilities correctly

## Implementation Timeline

**Week 1**: Phase 1 (Infrastructure)
**Week 2**: Phase 2 (Migration) + Phase 3 start
**Week 3**: Phase 3 completion + Phase 4
**Week 4**: Review, cleanup, and team training

## Notes

- Run tests frequently during migration to catch regressions
- Consider feature flags for gradual rollout
- Create example PRs showing the migration pattern
- Set up linting rules to enforce new patterns
- Consider automated migration scripts for simple cases

## Key Design Principles for Shared Mocks

1. **Framework Agnostic**: Mocks in @moatless/types should be pure functions without dependencies on vitest, jest, or any test framework
2. **Type Safety**: All mocks should return properly typed objects using types from @moatless/types
3. **Sensible Defaults**: Mocks should work without any configuration but allow overrides
4. **Consistency**: Same mock data structures used in both frontend and backend tests
5. **Separation of Concerns**:
   - Pure data mocks → @moatless/types/test-utils
   - Framework-specific mocks (with vi.fn()) → app-specific test directories

## Migration Benefits

1. **Code Reuse**: Frontend and backend tests share the same mock data
2. **Consistency**: Ensures both sides of the application test against the same data structures
3. **Maintainability**: Update mocks in one place when types change
4. **Type Safety**: TypeScript ensures mocks match actual types
5. **Discoverability**: Centralized location for all test utilities