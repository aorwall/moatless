# Screenshot Service Refactoring - Quick Reference

## Current Problems

```
ScreenshotOrchestrationService is a "middleman" service that:
1. Checks repository config
2. Manages server lifecycle
3. Waits for server ready
4. Captures screenshots
5. Updates task persistence
6. Emits events

This violates Single Responsibility Principle and exists only to avoid circular dependencies.
```

## Service Responsibilities (Current vs Proposed)

### ScreenshotOrchestrationService (CURRENT)
```
✓ Repository config checking     → MOVE to TaskIterationService
✓ Server orchestration           → MOVE to PlaywrightScreenshotService
✓ Server readiness checking      → MOVE to PlaywrightScreenshotService
✓ Screenshot capture delegation  → Already in PlaywrightScreenshotService
✓ Task persistence              → MOVE to TaskIterationService
✓ Event emission                → MOVE to TaskIterationService
```

### PlaywrightScreenshotService (PROPOSED)
```
NEW: Server readiness logic (ensureServerReady method)
NEW: Server ready waiting (waitForServerReady helper)
EXISTING: Page navigation
EXISTING: Screenshot capture via Playwright
EXISTING: File storage upload
```

### TaskIterationService (PROPOSED)
```
NEW: Repository config checking (shouldCaptureScreenshot method)
NEW: Screenshot orchestration call (captureScreenshotAndUpdate method)
NEW: Task persistence for screenshots
NEW: Event emission for screenshot updates
EXISTING: Git operations
EXISTING: Iteration lifecycle
```

## Dependency Changes

### Current (Problematic)
```
TaskIterationService
  ├─ ScreenshotOrchestrationService (middleman)
  │  ├─ PreviewOrchestratorService (server control)
  │  ├─ PlaywrightScreenshotService (actual capture)
  │  ├─ TaskRepository (for task updates)
  │  ├─ EventService (for events)
  │  └─ RepositoryService (for config)
  ├─ FileGateway
  ├─ EventService
  └─ RepositoryService
```

### Proposed (Clean)
```
TaskIterationService
  ├─ PlaywrightScreenshotService (direct dependency)
  │  ├─ PreviewOrchestratorService
  │  ├─ PlaywrightInteractionService
  │  └─ FileStorageService
  ├─ FileGateway
  ├─ EventService
  └─ RepositoryService

✓ No circular dependencies
✓ Clear parent-child relationships
✓ Eliminates middleman service
```

## Code Changes Summary

### 1. PlaywrightScreenshotService - ADD 3 new methods
```typescript
// Ensure server is running and ready before capture
async ensureServerReady(taskId: string, repository: Repository): Promise<string>

// Poll server until it's responsive
private async waitForServerReady(url: string, timeout: number): Promise<void>

// Updated signature - pass repository config, not just URL
async captureIterationScreenshot(
  taskId: string,
  iteration: TaskIteration,
  repository: Repository  // Changed from serverUrl: string
): Promise<{ screenshotFileId: string }>
```

### 2. TaskIterationService - ADD 3 new methods
```typescript
// Check if screenshots should be captured
private async shouldCaptureScreenshot(task: Task): Promise<boolean>

// Main orchestration - run in background
async captureScreenshotAndUpdate(
  task: Task,
  iteration: TaskIteration
): Promise<void>

// Handle task update and event emission
private async updateTaskAndEmitEvent(
  task: Task,
  iteration: TaskIteration,
  screenshotFileId: string
): Promise<void>
```

### 3. Update performPostProcessing() call
```typescript
// OLD
this.screenshotService
  .captureIterationScreenshotIfEnabled(task.taskId, iteration)
  .catch(error => { /* log */ })

// NEW
this.captureScreenshotAndUpdate(task, iteration)
  .catch(error => { /* log */ })
```

### 4. Update container.ts
```typescript
// REMOVE
container.bind(ScreenshotOrchestrationService).toSelf().inSingletonScope()

// ADD to PlaywrightScreenshotService dependencies
// (already has others, just add PreviewOrchestratorService)
```

### 5. Delete ScreenshotOrchestrationService
```
1. Delete file: screenshot-orchestration.service.ts
2. Remove from: preview/index.ts exports
3. Search for imports and verify none exist
```

## Implementation Order

1. **Phase 1**: Add methods to PlaywrightScreenshotService
   - Add PreviewOrchestratorService dependency
   - Implement ensureServerReady()
   - Implement waitForServerReady()
   - Update captureIterationScreenshot() signature

2. **Phase 2**: Add methods to TaskIterationService
   - Replace ScreenshotOrchestrationService with PlaywrightScreenshotService
   - Implement shouldCaptureScreenshot()
   - Implement captureScreenshotAndUpdate()
   - Implement updateTaskAndEmitEvent()

3. **Phase 3**: Update dependencies
   - Update container.ts bindings
   - Run tests to verify

4. **Phase 4**: Delete old code
   - Delete ScreenshotOrchestrationService
   - Remove exports
   - Final test run

## Testing Strategy

### New Unit Tests (PlaywrightScreenshotService)
- Test ensureServerReady() with server not started
- Test ensureServerReady() with dirty context + no auto-reload
- Test waitForServerReady() timeout
- Test waitForServerReady() success

### New Unit Tests (TaskIterationService)
- Test shouldCaptureScreenshot() - enabled
- Test shouldCaptureScreenshot() - disabled
- Test captureScreenshotAndUpdate() - happy path
- Test captureScreenshotAndUpdate() - error handling
- Test event emission

### Integration Tests
- End-to-end task execution → screenshot capture flow
- Verify events are emitted correctly
- Verify task is updated with screenshotFileId

## Files Affected

```
Modified:
  ✏️ apps/backend/src/features/preview/services/playwright-screenshot.service.ts
  ✏️ apps/backend/src/features/tasks/services/task-iteration.service.ts
  ✏️ apps/backend/src/container/container.ts
  ✏️ apps/backend/src/features/preview/index.ts
  ✏️ Test files (multiple)

Deleted:
  🗑️ apps/backend/src/features/preview/services/screenshot-orchestration.service.ts
```

## Key Benefits

```
✓ Eliminates middleman service (ScreenshotOrchestrationService)
✓ No circular dependencies
✓ Clearer separation of concerns
✓ Each service has single responsibility
✓ Easier to test (each service testable in isolation)
✓ Easier to understand (fewer indirections)
✓ Reduces coupling between features
```

## Risk Level: LOW

```
✓ PlaywrightScreenshotService only used by ScreenshotOrchestrationService
✓ No other consumers of ScreenshotOrchestrationService
✓ Changes are purely internal (no API changes to external code)
✓ Can be implemented incrementally with tests at each phase
✓ Easy to rollback if needed
```

