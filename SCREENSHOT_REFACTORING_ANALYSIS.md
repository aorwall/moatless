# Screenshot Service Architecture Analysis & Refactoring Proposal

## Current Architecture Overview

### Service Responsibilities (Current State)

#### ScreenshotOrchestrationService (23 KB)
**Current Role:** Central orchestrator breaking circular dependency
**Location:** `apps/backend/src/features/preview/services/screenshot-orchestration.service.ts`

**Public Methods:**
1. `captureIterationScreenshotIfEnabled(taskId, iteration)` - Entry point called from TaskIterationService
   - Checks if repository is found and screenshots are enabled
   - Delegates to `updateTaskWithScreenshot()` if enabled
   
2. `updateTaskWithScreenshot(taskId, iteration, agentType)` - Private, complex workflow
   - Checks if server has dirty context
   - Starts/restarts server if needed
   - Gets internal URL for Playwright
   - Waits for server to be ready
   - Calls PlaywrightScreenshotService.captureIterationScreenshot()
   - Updates task iteration with screenshot file ID
   - Saves task to repository
   - Emits screenshot update event

3. `waitForServerReady(url, timeout)` - Private helper
   - Polls HTTP HEAD requests until server responds
   - 30-second default timeout
   - 1-second retry interval

**Dependencies:**
- TaskRepository (for getting task and saving)
- EventService (for emitting screenshot update events)
- RepositoryService (for repository config)
- PreviewOrchestratorService (for server management)
- PlaywrightScreenshotService (for actual screenshot capture)

**Circular Dependency Issue:**
- ScreenshotOrchestrationService needs PreviewOrchestratorService
- PreviewOrchestratorService accesses ScreenshotOrchestrationService (or vice versa)
- **This service exists specifically to break that cycle**

---

#### PlaywrightScreenshotService (98 lines)
**Current Role:** Pure screenshot capture wrapper
**Location:** `apps/backend/src/features/preview/services/playwright-screenshot.service.ts`

**Public Methods:**
1. `captureIterationScreenshot(taskId, iteration, serverUrl)` - Main method
   - Checks if Playwright page exists (via PlaywrightInteractionService)
   - If not, navigates to "/" on the server
   - If yes, reuses existing page
   - Calls `takeSnapshotWithAutoRestart()`
   - Converts base64 to Buffer
   - Uploads to FileStorageService
   - Returns screenshotFileId

**Dependencies:**
- PlaywrightInteractionService (for Playwright page management)
- FileStorageService (for file upload)

**Scope:** Very focused - only handles screenshot capture and storage

---

#### TaskIterationService (523 lines)
**Current Role:** Iteration lifecycle manager
**Location:** `apps/backend/src/features/tasks/services/task-iteration.service.ts`

**Relevant Methods:**
1. `performPostProcessing(task, iteration)` - Calls screenshot service non-blocking
   ```typescript
   async performPostProcessing(task: Task, iteration: TaskIteration) {
     // ... git metadata, checkpoint, push ...
     // Screenshot capture runs in background (non-blocking)
     this.screenshotService
       .captureIterationScreenshotIfEnabled(task.taskId, iteration)
       .catch(error => { /* log error */ })
   }
   ```

**Dependencies:**
- FileGateway (for git operations)
- ScreenshotOrchestrationService (for screenshots)
- EventService (for iteration/task events)
- RepositoryService (for repository info)

**Current Integration:** Minimal - just calls `captureIterationScreenshotIfEnabled()` in background

---

### Data Flow (Current)

```
TaskExecutionCoordinator.finishIteration()
  ↓
TaskIterationService.performPostProcessing()
  ↓
ScreenshotOrchestrationService.captureIterationScreenshotIfEnabled()
  ├─ Check TaskRepository.get() → repository ID
  ├─ RepositoryService.getRepositoryById() → takeScreenshots enabled?
  ├─ [If enabled] updateTaskWithScreenshot():
  │  ├─ TaskRepository.get()
  │  ├─ RepositoryService.getRepositoryById() → autoReload setting
  │  ├─ PreviewOrchestratorService.runtime.isServerDirty() → isDirty
  │  ├─ PreviewOrchestratorService.getStatus() → server status
  │  ├─ [If not started] PreviewOrchestratorService.start()
  │  ├─ [If dirty + no autoReload] PreviewOrchestratorService.restart()
  │  ├─ PreviewOrchestratorService.getInternalUrl()
  │  ├─ waitForServerReady(internalUrl) → polls HTTP
  │  ├─ PlaywrightScreenshotService.captureIterationScreenshot()
  │  │  ├─ PlaywrightInteractionService.getCurrentUrl()
  │  │  ├─ [If no page] PlaywrightInteractionService.navigateTo("/")
  │  │  ├─ PlaywrightInteractionService.takeSnapshotWithAutoRestart()
  │  │  └─ FileStorageService.uploadFile()
  │  ├─ TaskRepository.save() → update iteration with screenshotFileId
  │  └─ EventService.emitEvent() → screenshot update event
  └─ [Errors logged but not rethrown - non-blocking]
```

---

## Problems with Current Architecture

### 1. **Circular Dependency Avoidance Pattern**
- ScreenshotOrchestrationService exists primarily to break a circular dependency
- This is a code smell - suggests responsibilities are misaligned
- Service is a "middleman" with no clear primary responsibility

### 2. **ScreenshotOrchestrationService Mixes Too Many Concerns**
| Concern | Location | Responsibility |
|---------|----------|-----------------|
| Repository config checking | Lines 40-58 | Should be TaskIterationService or a config service |
| Server orchestration | Lines 95-108 | Should stay in PreviewOrchestratorService |
| Server readiness check | Lines 123-128 | Should be in PreviewOrchestratorService or PlaywrightScreenshotService |
| Screenshot capture | Line 135-139 | Should be PlaywrightScreenshotService |
| Task persistence | Lines 147-158 | Should be TaskIterationService |
| Event emission | Lines 160-180 | Should be TaskIterationService |

### 3. **Testability Issues**
- Large service with many dependencies makes unit testing complex
- Mixed concerns means you can't test just one thing
- Server orchestration logic (start/restart) is tested in screenshot service tests, not preview tests

### 4. **Separation of Concerns Violations**
```
Ideal:
  TaskIterationService → orchestrates task lifecycle, emits events
  ScreenshotOrchestrationService → SHOULD NOT EXIST (responsibility conflict)
  PlaywrightScreenshotService → captures screenshots, uploads files
  PreviewOrchestratorService → manages server lifecycle (start/restart/status)

Current:
  TaskIterationService → calls screenshot service
  ScreenshotOrchestrationService → does everything (bad)
  PlaywrightScreenshotService → only does screenshot capture (limited)
  PreviewOrchestratorService → server lifecycle, but screenshot service calls its internals
```

### 5. **Code Duplication Risk**
- `isServerDirty()` and `getStatus()` calls are in ScreenshotOrchestrationService
- Same logic might be needed elsewhere (preview routes, other features)
- No public API on PreviewOrchestratorService for these concerns (uses `.runtime` directly)

### 6. **Non-blocking Call Pattern Issues**
```typescript
// TaskIterationService calls this without awaiting
this.screenshotService
  .captureIterationScreenshotIfEnabled(taskId, iteration)
  .catch(error => { /* log */ })
```
- Screenshot failures don't affect task completion (good)
- But screenshot failures are only logged, not tracked
- No way to know screenshot failed without checking logs
- Could lead to missing screenshots without visibility

---

## Proposed Refactoring

### Step 1: Move Repository Config Logic → TaskIterationService
**Why:** TaskIterationService already has task context and calls screenshot service

**Changes:**
```typescript
// TaskIterationService.performPostProcessing()
const shouldCaptureScreenshot = await this.shouldCaptureScreenshot(task)
if (shouldCaptureScreenshot) {
  this.captureScreenshotAsync(task, iteration)
}

private async shouldCaptureScreenshot(task: Task): Promise<boolean> {
  const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
  return repository?.previewConfig?.takeScreenshots ?? false
}

private captureScreenshotAsync(task: Task, iteration: TaskIteration): void {
  this.playwrightScreenshotService
    .captureIterationScreenshot(...)
    .catch(error => { /* log */ })
}
```

**Benefits:**
- TaskIterationService owns all task-related decisions
- Reduces ScreenshotOrchestrationService surface
- Clearer responsibility: task iteration → screenshot decision

---

### Step 2: Move Server Orchestration to PlaywrightScreenshotService
**Why:** PlaywrightScreenshotService already knows it needs a running server with valid URL

**Changes:**
```typescript
// PlaywrightScreenshotService
async captureIterationScreenshot(
  taskId: string,
  iteration: TaskIteration,
  repository: Repository,  // Pass config, not just URL
): Promise<{ screenshotFileId: string }> {
  // Ensure server is ready before capturing
  const internalUrl = await this.ensureServerReady(taskId, repository)
  
  // ... rest of screenshot capture using internalUrl
}

private async ensureServerReady(
  taskId: string,
  repository: Repository,
): Promise<string> {
  const isDirty = await this.previewOrchestrator.runtime.isServerDirty(taskId)
  const autoReload = repository.previewConfig?.autoReload ?? false
  
  const serverStatus = await this.previewOrchestrator.getStatus(taskId)
  
  if (serverStatus.status !== 'started') {
    await this.previewOrchestrator.start(taskId)
  } else if (isDirty && !autoReload) {
    await this.previewOrchestrator.restart(taskId)
  }
  
  // Wait for server ready
  await this.waitForServerReady(serverStatus.url, 30000)
  
  const internalUrl = this.previewOrchestrator.getInternalUrl(taskId)
  if (!internalUrl) {
    throw new Error('No internal URL available')
  }
  
  return internalUrl
}

private async waitForServerReady(url: string, timeout: number): Promise<void> {
  // Move from ScreenshotOrchestrationService
}
```

**Benefits:**
- PlaywrightScreenshotService is self-contained
- Can be tested independently with mocked PreviewOrchestratorService
- No need for intermediary service
- Clearer dependencies: Playwright service → Preview orchestrator

---

### Step 3: Move Task Persistence & Events → TaskIterationService
**Why:** TaskIterationService is responsible for iteration state and events

**Changes:**
```typescript
// TaskIterationService
async captureScreenshotAndUpdate(
  task: Task,
  iteration: TaskIteration,
): Promise<void> {
  try {
    const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
    if (!repository?.previewConfig?.takeScreenshots) {
      return
    }
    
    // Call updated PlaywrightScreenshotService
    const { screenshotFileId } = await this.playwrightScreenshotService
      .captureIterationScreenshot(task.taskId, iteration, repository)
    
    // TaskIterationService now owns saving and event emission
    iteration.screenshotFileId = screenshotFileId
    await this.updateTaskAndEmitEvent(task, iteration, screenshotFileId)
  } catch (error) {
    logger.warn('Failed to capture screenshot', { error })
    // Don't fail iteration
  }
}

private async updateTaskAndEmitEvent(
  task: Task,
  iteration: TaskIteration,
  screenshotFileId: string,
): Promise<void> {
  // Save task (already done elsewhere, but ensure it's saved)
  // Emit screenshot update event
  await this.eventService.emitEvent({
    taskId: task.taskId,
    type: 'taskIteration',
    subType: 'updated',
    agentType: task.agentType,
    iterationId: iteration.iterationNumber,
    payload: {
      status: iteration.status,
      previousStatus: iteration.status,
      iterationNumber: iteration.iterationNumber,
      screenshotFileId,
    },
    eventId: randomUUID(),
    timestamp: new Date(),
    isError: false,
  })
}
```

**Benefits:**
- All task iteration state changes in one place
- Clear event emission ownership
- Easier to track iteration lifecycle

---

### Step 4: Delete ScreenshotOrchestrationService
**Why:** Responsibilities distributed to more appropriate services

**Changes:**
- Remove from container.ts bindings
- Update TaskIterationService to inject PlaywrightScreenshotService directly
- Update tests

---

## New Architecture After Refactoring

```
Responsibility Matrix:
┌─────────────────────────────────────────────────────────────────┐
│ Service                       │ Responsibilities                 │
├─────────────────────────────────────────────────────────────────┤
│ TaskIterationService          │ - Iteration lifecycle            │
│                               │ - Screenshot decision (config)   │
│                               │ - Task persistence               │
│                               │ - Event emission                 │
├─────────────────────────────────────────────────────────────────┤
│ PlaywrightScreenshotService   │ - Server readiness checks       │
│                               │ - Server start/restart logic    │
│                               │ - Screenshot capture            │
│                               │ - File upload                    │
├─────────────────────────────────────────────────────────────────┤
│ PreviewOrchestratorService    │ - Server lifecycle (abstract)    │
│                               │ - Status management              │
│                               │ - URL resolution                 │
├─────────────────────────────────────────────────────────────────┤
│ ScreenshotOrchestrationService│ ❌ DELETED                       │
└─────────────────────────────────────────────────────────────────┘

New Data Flow:
TaskExecutionCoordinator.finishIteration()
  ↓
TaskIterationService.performPostProcessing()
  ├─ [Check if screenshots enabled]
  ├─ [Call PlaywrightScreenshotService async]
  └─ TaskIterationService.captureScreenshotAndUpdate() [background]
      ├─ PlaywrightScreenshotService.captureIterationScreenshot()
      │  ├─ PreviewOrchestratorService.runtime.isServerDirty()
      │  ├─ PreviewOrchestratorService.getStatus()
      │  ├─ PreviewOrchestratorService.start/restart()
      │  ├─ Wait for server ready
      │  ├─ PlaywrightInteractionService → Playwright
      │  └─ FileStorageService → upload
      └─ TaskIterationService.updateTaskAndEmitEvent()
         └─ EventService.emitEvent()
```

---

## Implementation Plan

### Phase 1: Add New Methods to PlaywrightScreenshotService
- Add `ensureServerReady()` method
- Add `waitForServerReady()` helper
- Update `captureIterationScreenshot()` signature to accept repository
- Add necessary dependencies (PreviewOrchestratorService)

### Phase 2: Add New Methods to TaskIterationService
- Add `shouldCaptureScreenshot()` method
- Add `captureScreenshotAndUpdate()` method
- Update `performPostProcessing()` to call new method
- Inject PlaywrightScreenshotService instead of ScreenshotOrchestrationService

### Phase 3: Update Container Bindings
- Remove ScreenshotOrchestrationService binding
- Ensure all services have proper dependencies

### Phase 4: Update Tests
- Move ScreenshotOrchestrationService tests to PlaywrightScreenshotService
- Update TaskIterationService tests
- Update all integration tests

### Phase 5: Delete ScreenshotOrchestrationService
- Delete file
- Delete from preview/index.ts exports
- Verify no other imports exist

---

## Testing Strategy

### Unit Tests (New)

**PlaywrightScreenshotService:**
```typescript
// Mock PreviewOrchestratorService
// Mock FileStorageService
// Test ensureServerReady() with various server states
// Test screenshot capture
// Test file upload
```

**TaskIterationService:**
```typescript
// Test shouldCaptureScreenshot() with config variations
// Test captureScreenshotAndUpdate() happy path
// Test error handling
// Test event emission
```

### Integration Tests
```typescript
// Test full flow from task execution → screenshot capture
// Test with real preview server (if needed)
```

---

## Dependency Graph (After Refactoring)

```
TaskIterationService
  ├── PlaywrightScreenshotService
  │   ├── PlaywrightInteractionService
  │   ├── FileStorageService
  │   └── PreviewOrchestratorService (direct dependency)
  ├── EventService
  ├── RepositoryService
  └── FileGateway

PreviewOrchestratorService (no circular dependency!)
  ├── PreviewRuntimeService
  ├── RepositoryService
  ├── TaskRepository
  └── PlaywrightInteractionService
```

**Key Improvement:** No circular dependencies, clean parent-child relationships

---

## Migration Checklist

### Code Changes
- [ ] Add methods to PlaywrightScreenshotService
- [ ] Update PlaywrightScreenshotService constructor (add PreviewOrchestratorService)
- [ ] Move `waitForServerReady()` to PlaywrightScreenshotService
- [ ] Add methods to TaskIterationService
- [ ] Update TaskIterationService constructor (replace ScreenshotOrchestrationService with PlaywrightScreenshotService)
- [ ] Update TaskIterationService.performPostProcessing()
- [ ] Update container.ts bindings
- [ ] Delete ScreenshotOrchestrationService file
- [ ] Update preview/index.ts exports

### Tests
- [ ] Update TaskIterationService tests
- [ ] Move ScreenshotOrchestrationService tests to PlaywrightScreenshotService
- [ ] Update integration tests
- [ ] Run full test suite

### Documentation
- [ ] Update service documentation
- [ ] Update architecture diagrams
- [ ] Update CLAUDE.md if needed

---

## Risk Assessment

### Low Risk
- PlaywrightScreenshotService is currently used only by ScreenshotOrchestrationService
- No other consumers of ScreenshotOrchestrationService exist (checked codebase)
- Changes are additive first, then removal

### Medium Risk
- Server orchestration logic moving to PlaywrightScreenshotService (but it's focused logic)
- Event emission moving to TaskIterationService (but it's a clean addition)

### Mitigation
- Keep ScreenshotOrchestrationService in place during refactoring (backward compatible)
- Add new methods in parallel
- Update call sites gradually
- Run full integration tests after each phase

---

## Files to Modify

### Primary Files
1. `/apps/backend/src/features/preview/services/playwright-screenshot.service.ts`
   - Add `ensureServerReady()` method
   - Add `waitForServerReady()` helper
   - Add PreviewOrchestratorService dependency
   - Update `captureIterationScreenshot()` signature

2. `/apps/backend/src/features/tasks/services/task-iteration.service.ts`
   - Add `shouldCaptureScreenshot()` method
   - Add `captureScreenshotAndUpdate()` method
   - Update `performPostProcessing()` call
   - Replace ScreenshotOrchestrationService with PlaywrightScreenshotService
   - Add event emission logic

3. `/apps/backend/src/container/container.ts`
   - Remove ScreenshotOrchestrationService binding
   - Ensure PlaywrightScreenshotService has PreviewOrchestratorService injected

### Secondary Files
4. `/apps/backend/src/features/preview/services/screenshot-orchestration.service.ts`
   - DELETE after migration complete

5. `/apps/backend/src/features/preview/index.ts`
   - Remove ScreenshotOrchestrationService export

6. Test files (multiple)
   - Update all test fixtures and mocks

---

## Conclusion

This refactoring eliminates a service that exists only to avoid circular dependencies and redistributes its responsibilities to more appropriate locations:

- **Repository config checking** → TaskIterationService (owns iteration decisions)
- **Server readiness** → PlaywrightScreenshotService (needs the server)
- **Task persistence & events** → TaskIterationService (owns iteration state)
- **ScreenshotOrchestrationService** → DELETE (no longer needed)

Result: Cleaner architecture, better testability, clearer responsibilities, and no circular dependencies.

