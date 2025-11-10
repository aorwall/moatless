# Screenshot Service Refactoring - Implementation Code Guide

## Phase 1: Update PlaywrightScreenshotService

### File: `apps/backend/src/features/preview/services/playwright-screenshot.service.ts`

#### Step 1a: Add New Imports and Dependency
```typescript
import type { Repository } from '@moatless/types'  // Add this import
import { PreviewOrchestratorService } from './preview-orchestrator.service'  // Add

@injectable()
export class PlaywrightScreenshotService {
  private serviceLogger = logger.child({ service: 'playwright-screenshot' })

  constructor(
    @inject(PlaywrightInteractionService)
    private readonly playwrightInteraction: PlaywrightInteractionService,
    @inject(FileStorageService) private readonly fileStorage: FileStorageService,
    @inject(PreviewOrchestratorService)  // ADD THIS
    private readonly previewOrchestrator: PreviewOrchestratorService,
  ) {}
```

#### Step 1b: Update Main Method Signature
```typescript
// OLD signature
async captureIterationScreenshot(
  taskId: string,
  iteration: TaskIteration,
  serverUrl: string,
): Promise<{ screenshotFileId: string }> {

// NEW signature
async captureIterationScreenshot(
  taskId: string,
  iteration: TaskIteration,
  repository: Repository,  // Changed from serverUrl
): Promise<{ screenshotFileId: string }> {
```

#### Step 1c: Update Method Implementation
```typescript
async captureIterationScreenshot(
  taskId: string,
  iteration: TaskIteration,
  repository: Repository,
): Promise<{ screenshotFileId: string }> {
  const taskLogger = this.serviceLogger.child({
    taskId,
    iterationNumber: iteration.iterationNumber,
  })

  try {
    // NEW: Ensure server is ready before capturing
    const internalUrl = await this.ensureServerReady(taskId, repository)
    
    // Check if Playwright page already exists
    const currentUrl = this.playwrightInteraction.getCurrentUrl(taskId)

    if (!currentUrl) {
      // No existing page - create one by navigating to "/"
      taskLogger.info('No existing Playwright page, navigating to /', { internalUrl })
      await this.playwrightInteraction.navigateTo(taskId, '/', internalUrl)
    } else {
      // Page exists - use it at current location
      taskLogger.info('Reusing existing Playwright page', {
        currentUrl,
        note: 'Preserving agent browser state',
      })
    }

    // Capture screenshot with auto-restart capability
    taskLogger.info('Capturing screenshot via PlaywrightInteractionService')
    const snapshotResult = await this.playwrightInteraction.takeSnapshotWithAutoRestart(
      taskId,
      true, // includeScreenshot
    )

    if (!snapshotResult.screenshotBase64) {
      throw new Error('Screenshot capture returned no base64 data')
    }

    // Convert base64 to Buffer
    const screenshotBuffer = Buffer.from(snapshotResult.screenshotBase64, 'base64')
    taskLogger.debug('Screenshot captured', {
      base64Length: snapshotResult.screenshotBase64.length,
      bufferSize: screenshotBuffer.length,
    })

    // Upload to file storage
    const filename = `task-${taskId}-iter-${iteration.iterationNumber}.png`
    taskLogger.info('Uploading screenshot to file storage', { filename })

    const screenshotFile = await this.fileStorage.uploadFile(
      screenshotBuffer,
      filename,
      'image/png',
    )

    taskLogger.info('Screenshot uploaded successfully', {
      screenshotFileId: screenshotFile.fileId,
    })

    return {
      screenshotFileId: screenshotFile.fileId,
    }
  } catch (error) {
    taskLogger.error('Failed to capture screenshot', error as Error)
    throw error
  }
}
```

#### Step 1d: Add ensureServerReady() Method
```typescript
/**
 * Ensure preview server is running and ready for screenshot capture.
 * Handles server start/restart based on dirty context and auto-reload settings.
 *
 * @param taskId - Task ID
 * @param repository - Repository configuration
 * @returns Internal URL for Playwright access
 * @throws Error if server fails to start or becomes unavailable
 */
private async ensureServerReady(
  taskId: string,
  repository: Repository,
): Promise<string> {
  const taskLogger = this.serviceLogger.child({ taskId })

  try {
    // Check if server has dirty context (code changes made)
    const isDirty = await this.previewOrchestrator.runtime.isServerDirty(taskId)
    const autoReload = repository.previewConfig?.autoReload ?? false

    // Get current server status
    const serverStatus = await this.previewOrchestrator.getStatus(taskId)

    if (serverStatus.status !== 'started') {
      taskLogger.info('Server not started, starting now', {
        currentStatus: serverStatus.status,
      })
      await this.previewOrchestrator.start(taskId)
    } else if (isDirty && !autoReload) {
      // No auto-reload and server is dirty - restart will apply changes
      taskLogger.info('Code changes detected without auto-reload - restarting server')
      await this.previewOrchestrator.restart(taskId)
    }

    // Verify server is now started
    const finalStatus = await this.previewOrchestrator.getStatus(taskId)
    if (finalStatus.status !== 'started' || !finalStatus.url) {
      taskLogger.warn('Server failed to start for screenshot')
      throw new Error('Preview server failed to start')
    }

    // Get internal URL for Playwright (bypasses authentication)
    const internalUrl = this.previewOrchestrator.getInternalUrl(taskId)
    if (!internalUrl) {
      taskLogger.warn('No internal URL available for screenshot')
      throw new Error('No internal URL available for preview server')
    }

    // Wait for server to be responsive
    taskLogger.debug('Waiting for server to be ready', {
      internalUrl,
      externalUrl: finalStatus.url,
    })
    await this.waitForServerReady(internalUrl, 30000) // 30 second timeout

    taskLogger.info('Server is ready for screenshot', { internalUrl })
    return internalUrl
  } catch (error) {
    taskLogger.error('Failed to ensure server ready', error as Error)
    throw error
  }
}
```

#### Step 1e: Add waitForServerReady() Helper
```typescript
/**
 * Wait for preview server to be ready (responsive to HTTP requests)
 *
 * @param url - Server URL to poll
 * @param timeout - Maximum time to wait in milliseconds
 * @throws Error if server doesn't become ready within timeout
 */
private async waitForServerReady(url: string, timeout = 30000): Promise<void> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeout) {
    try {
      const response = await fetch(url, {
        method: 'HEAD',
        signal: AbortSignal.timeout(5000),
      })

      if (response.ok || response.status < 500) {
        // Server is responding
        this.serviceLogger.debug('Server is ready', { url, status: response.status })
        return
      }

      this.serviceLogger.debug('Server responded with error status', {
        url,
        status: response.status,
      })
    } catch (error) {
      this.serviceLogger.debug('Server not ready yet', {
        url,
        error: error instanceof Error ? error.message : String(error),
      })
    }

    // Wait 1 second before retry
    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  throw new Error(`Server at ${url} did not become ready within ${timeout}ms`)
}
```

---

## Phase 2: Update TaskIterationService

### File: `apps/backend/src/features/tasks/services/task-iteration.service.ts`

#### Step 2a: Replace Import and Dependency
```typescript
// OLD import
import { ScreenshotOrchestrationService } from '../../preview/services/screenshot-orchestration.service'

// NEW import
import { PlaywrightScreenshotService } from '../../preview/services/playwright-screenshot.service'

// OLD constructor
constructor(
  @inject(FileGateway) private readonly fileGateway: FileGateway,
  @inject(ScreenshotOrchestrationService)
  private readonly screenshotService: ScreenshotOrchestrationService,
  @inject(EventService) private readonly eventService: EventService,
  @inject(RepositoryService) private readonly repositoryService: RepositoryService,
)

// NEW constructor
constructor(
  @inject(FileGateway) private readonly fileGateway: FileGateway,
  @inject(PlaywrightScreenshotService)
  private readonly playwrightScreenshotService: PlaywrightScreenshotService,
  @inject(EventService) private readonly eventService: EventService,
  @inject(RepositoryService) private readonly repositoryService: RepositoryService,
) {}
```

#### Step 2b: Update performPostProcessing() Method
```typescript
/**
 * Perform all post-processing steps for an iteration:
 * 1. Capture git metadata
 * 2. Create checkpoint (if enabled)
 * 3. Push changes (if enabled)
 * 4. Capture commit hash (fallback)
 * 5. Trigger screenshot capture (async, non-blocking)
 *
 * @param task - Task containing the iteration
 * @param iteration - Iteration to post-process
 */
async performPostProcessing(task: Task, iteration: TaskIteration): Promise<void> {
  const taskLogger = logger.child({
    taskId: task.taskId,
    iterationNumber: iteration.iterationNumber,
  })

  await this.captureGitMetadata(task.taskId, iteration)

  if (task.gitCheckpointEnabled) {
    await this.createCheckpoint(task, iteration)
  }

  if (task.gitPushEnabled) {
    await this.pushChanges(task, iteration)
  }

  await this.captureCommitHashIfNeeded(task.taskId, iteration)

  // Screenshot capture runs in background (non-blocking)
  // Changed from calling screenshotService to direct method
  this.captureScreenshotAndUpdate(task, iteration)
    .catch((error) => {
      taskLogger.warn('Failed to capture preview screenshot', {
        error: error instanceof Error ? error.message : String(error),
        iterationNumber: iteration.iterationNumber,
      })
    })
}
```

#### Step 2c: Add shouldCaptureScreenshot() Method
```typescript
/**
 * Check if screenshots should be captured for this task
 *
 * @param task - Task to check
 * @returns true if screenshots are enabled in repository config
 */
private async shouldCaptureScreenshot(task: Task): Promise<boolean> {
  try {
    const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
    if (!repository) {
      return false
    }

    return repository.previewConfig?.takeScreenshots ?? false
  } catch (error) {
    logger.warn('Failed to check screenshot setting', {
      taskId: task.taskId,
      error: error instanceof Error ? error.message : String(error),
    })
    return false
  }
}
```

#### Step 2d: Add captureScreenshotAndUpdate() Method
```typescript
/**
 * Capture screenshot and update task with result
 * This is the main orchestration method that coordinates screenshot capture,
 * task updates, and event emission.
 *
 * @param task - Task to capture screenshot for
 * @param iteration - Iteration to attach screenshot to
 */
async captureScreenshotAndUpdate(task: Task, iteration: TaskIteration): Promise<void> {
  const taskLogger = logger.child({
    taskId: task.taskId,
    iterationNumber: iteration.iterationNumber,
  })

  try {
    // Check if screenshots are enabled
    const shouldCapture = await this.shouldCaptureScreenshot(task)
    if (!shouldCapture) {
      taskLogger.debug('Screenshots disabled for repository, skipping capture')
      return
    }

    // Get repository configuration for server settings
    const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
    if (!repository) {
      taskLogger.warn('Repository not found for screenshot capture')
      return
    }

    // Capture screenshot via PlaywrightScreenshotService
    taskLogger.info('Capturing iteration screenshot')
    const { screenshotFileId } = await this.playwrightScreenshotService
      .captureIterationScreenshot(task.taskId, iteration, repository)

    if (screenshotFileId) {
      // Update task with screenshot
      await this.updateTaskAndEmitEvent(task, iteration, screenshotFileId)
    } else {
      taskLogger.debug('No screenshot file ID returned')
    }
  } catch (error) {
    taskLogger.error('Failed to capture screenshot', error as Error)
    // Don't rethrow - this is non-blocking post-processing
  }
}
```

#### Step 2e: Add updateTaskAndEmitEvent() Method
```typescript
/**
 * Update task with screenshot file ID and emit event
 *
 * @param task - Task to update
 * @param iteration - Iteration with screenshot
 * @param screenshotFileId - Screenshot file ID to attach
 */
private async updateTaskAndEmitEvent(
  task: Task,
  iteration: TaskIteration,
  screenshotFileId: string,
): Promise<void> {
  const taskLogger = logger.child({
    taskId: task.taskId,
    iterationNumber: iteration.iterationNumber,
    screenshotFileId,
  })

  try {
    // Update iteration with screenshot file ID
    iteration.screenshotFileId = screenshotFileId
    taskLogger.info('Screenshot file ID added to iteration', { screenshotFileId })

    // Emit event for screenshot capture
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

    taskLogger.info('Emitted screenshot update event')
  } catch (error) {
    taskLogger.error('Failed to emit screenshot event', error as Error)
    // Don't rethrow - screenshot was captured, just event emission failed
  }
}
```

---

## Phase 3: Update Container Bindings

### File: `apps/backend/src/container/container.ts`

#### Step 3a: Remove ScreenshotOrchestrationService Binding

Find this line in the `bindServices()` function:
```typescript
// FIND and REMOVE this line
container.bind(ScreenshotOrchestrationService).toSelf().inSingletonScope()
```

The PlaywrightScreenshotService binding already exists and now has the necessary dependencies.

#### Step 3b: Verify PlaywrightScreenshotService Dependencies

The binding should remain as-is (it's already in the file):
```typescript
container.bind(PlaywrightScreenshotService).toSelf().inSingletonScope()
```

Since we added PreviewOrchestratorService to PlaywrightScreenshotService constructor,
and both are already bound to the container, InversifyJS will automatically inject it.

---

## Phase 4: Update Exports

### File: `apps/backend/src/features/preview/index.ts`

#### Step 4a: Remove ScreenshotOrchestrationService Export

Find and remove this line:
```typescript
export { ScreenshotOrchestrationService } from './services/screenshot-orchestration.service'
```

---

## Phase 5: Update Tests

### File: `apps/backend/src/features/preview/services/__tests__/playwright-screenshot.service.test.ts`

Create new test file with these test cases:

```typescript
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import type { Repository, Task, TaskIteration } from '@moatless/types'
import { Container } from 'inversify'
import { PlaywrightScreenshotService } from '../playwright-screenshot.service'
import { PlaywrightInteractionService } from '../playwright-interaction.service'
import { FileStorageService } from '../../file-storage/services/file-storage.service'
import { PreviewOrchestratorService } from '../preview-orchestrator.service'

describe('PlaywrightScreenshotService', () => {
  let service: PlaywrightScreenshotService
  let mockPlaywright: any
  let mockFileStorage: any
  let mockPreviewOrchestrator: any

  const mockRepository: Repository = {
    // Fill with test data
    previewConfig: { autoReload: false, takeScreenshots: true },
  }

  const mockIteration: TaskIteration = {
    iterationNumber: 1,
    status: 'completed',
    // ... other fields
  }

  beforeEach(() => {
    mockPlaywright = {
      getCurrentUrl: vi.fn().mockReturnValue('http://localhost:3000'),
      takeSnapshotWithAutoRestart: vi.fn().mockResolvedValue({
        screenshotBase64: 'base64data',
      }),
      navigateTo: vi.fn().mockResolvedValue(undefined),
    }

    mockFileStorage = {
      uploadFile: vi.fn().mockResolvedValue({
        fileId: 'file-123',
      }),
    }

    mockPreviewOrchestrator = {
      runtime: {
        isServerDirty: vi.fn().mockResolvedValue(false),
      },
      getStatus: vi.fn().mockResolvedValue({
        status: 'started',
        url: 'http://localhost:3000',
      }),
      getInternalUrl: vi.fn().mockReturnValue('http://localhost:3000'),
      start: vi.fn().mockResolvedValue(undefined),
      restart: vi.fn().mockResolvedValue(undefined),
    }

    const container = new Container()
    container.bind(PlaywrightInteractionService).toConstantValue(mockPlaywright)
    container.bind(FileStorageService).toConstantValue(mockFileStorage)
    container.bind(PreviewOrchestratorService).toConstantValue(mockPreviewOrchestrator)
    container.bind(PlaywrightScreenshotService).toSelf()

    service = container.get(PlaywrightScreenshotService)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('captureIterationScreenshot', () => {
    it('should capture screenshot successfully', async () => {
      const result = await service.captureIterationScreenshot(
        'task-123',
        mockIteration,
        mockRepository,
      )

      expect(result.screenshotFileId).toBe('file-123')
      expect(mockFileStorage.uploadFile).toHaveBeenCalled()
    })
  })

  describe('ensureServerReady', () => {
    it('should start server if not running', async () => {
      mockPreviewOrchestrator.getStatus.mockResolvedValueOnce({
        status: 'stopped',
        url: undefined,
      })

      // Would need to make ensureServerReady public or test through captureIterationScreenshot
      const result = await service.captureIterationScreenshot(
        'task-123',
        mockIteration,
        mockRepository,
      )

      expect(mockPreviewOrchestrator.start).toHaveBeenCalled()
    })

    it('should restart server if dirty and no auto-reload', async () => {
      mockPreviewOrchestrator.runtime.isServerDirty.mockResolvedValueOnce(true)
      mockRepository.previewConfig!.autoReload = false

      const result = await service.captureIterationScreenshot(
        'task-123',
        mockIteration,
        mockRepository,
      )

      expect(mockPreviewOrchestrator.restart).toHaveBeenCalled()
    })
  })
})
```

### Update TaskIterationService Tests

Update `apps/backend/src/features/tasks/services/__tests__/task-iteration.service.test.ts`:

```typescript
// Replace the ScreenshotOrchestrationService mock
// OLD
mockScreenshotService = {
  captureIterationScreenshotIfEnabled: vi.fn(),
}
container.bind(ScreenshotOrchestrationService).toConstantValue(mockScreenshotService)

// NEW
mockPlaywrightScreenshotService = {
  captureIterationScreenshot: vi.fn().mockResolvedValue({
    screenshotFileId: 'file-123',
  }),
}
container.bind(PlaywrightScreenshotService).toConstantValue(mockPlaywrightScreenshotService)

// Update constructor injection
container.bind(TaskIterationService).toSelf()
```

---

## Summary of Code Changes

### Added to PlaywrightScreenshotService
- Constructor parameter: `PreviewOrchestratorService`
- New method: `ensureServerReady(taskId, repository): Promise<string>`
- New private method: `waitForServerReady(url, timeout): Promise<void>`
- Updated method signature: `captureIterationScreenshot()` now accepts `repository` instead of `serverUrl`

### Added to TaskIterationService
- Constructor change: `ScreenshotOrchestrationService` → `PlaywrightScreenshotService`
- New private method: `shouldCaptureScreenshot(task): Promise<boolean>`
- New method: `captureScreenshotAndUpdate(task, iteration): Promise<void>`
- New private method: `updateTaskAndEmitEvent(task, iteration, screenshotFileId): Promise<void>`
- Updated method: `performPostProcessing()` to call new method instead of old service

### Container Changes
- Remove: `ScreenshotOrchestrationService` binding
- PlaywrightScreenshotService already bound, will auto-inject new dependency

### File Deletions
- Delete: `screenshot-orchestration.service.ts`
- Remove export from: `preview/index.ts`

