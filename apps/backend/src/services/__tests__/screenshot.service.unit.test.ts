import type { Task, TaskIteration } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ScreenshotService } from '../screenshot.service.js'

describe('ScreenshotService', () => {
  let screenshotService: ScreenshotService
  let mockGetTask: ReturnType<typeof vi.fn>
  let mockSaveTask: ReturnType<typeof vi.fn>
  let mockEmitEvent: ReturnType<typeof vi.fn>
  let mockGetRepositoryById: ReturnType<typeof vi.fn>
  let mockCapturePreviewScreenshot: ReturnType<typeof vi.fn>

  beforeEach(() => {
    // Create mock functions
    mockGetTask = vi.fn()
    mockSaveTask = vi.fn()
    mockEmitEvent = vi.fn()
    mockGetRepositoryById = vi.fn()
    mockCapturePreviewScreenshot = vi.fn()

    // Create mock storage
    const mockStorage = {
      getTask: mockGetTask,
      saveTask: mockSaveTask,
    } as any

    // Create mock event service
    const mockEventService = {
      emitEvent: mockEmitEvent,
    } as any

    // Create mock repository service
    const mockRepositoryService = {
      getRepositoryById: mockGetRepositoryById,
    } as any

    // Create mock workspace service
    const mockWorkspaceService = {
      capturePreviewScreenshot: mockCapturePreviewScreenshot,
    } as any

    // Create screenshot service instance
    screenshotService = new ScreenshotService(
      mockStorage,
      mockEventService,
      mockRepositoryService,
      mockWorkspaceService,
    )
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('captureIterationScreenshotIfEnabled', () => {
    const taskId = 'test-task-123'
    const iteration: TaskIteration = {
      iterationNumber: 1,
      startTime: new Date(),
      status: 'completed',
      messageCount: 1,
      costUsd: 0,
    }

    it('should skip screenshot capture if task not found', async () => {
      mockGetTask.mockResolvedValue(null)

      await screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration)

      expect(mockGetTask).toHaveBeenCalledWith(taskId)
      expect(mockGetRepositoryById).not.toHaveBeenCalled()
      expect(mockCapturePreviewScreenshot).not.toHaveBeenCalled()
    })

    it('should skip screenshot capture if repository not found', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(null)

      await screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration)

      expect(mockGetTask).toHaveBeenCalledWith(taskId)
      expect(mockGetRepositoryById).toHaveBeenCalledWith('repo-123')
      expect(mockCapturePreviewScreenshot).not.toHaveBeenCalled()
    })

    it('should skip screenshot capture if screenshots are disabled', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const repository = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        createdAt: new Date(),
        updatedAt: new Date(),
        codeExecution: { type: 'local' as const },
        previewConfig: {
          takeScreenshots: false,
        },
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(repository)

      await screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration)

      expect(mockGetTask).toHaveBeenCalledWith(taskId)
      expect(mockGetRepositoryById).toHaveBeenCalledWith('repo-123')
      expect(mockCapturePreviewScreenshot).not.toHaveBeenCalled()
    })

    it('should skip screenshot capture if previewConfig is undefined', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const repository = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        createdAt: new Date(),
        updatedAt: new Date(),
        codeExecution: { type: 'local' as const },
        // No previewConfig
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(repository)

      await screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration)

      expect(mockGetTask).toHaveBeenCalledWith(taskId)
      expect(mockGetRepositoryById).toHaveBeenCalledWith('repo-123')
      expect(mockCapturePreviewScreenshot).not.toHaveBeenCalled()
    })

    it('should capture screenshot when enabled', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const repository = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        createdAt: new Date(),
        updatedAt: new Date(),
        codeExecution: { type: 'local' as const },
        previewConfig: {
          takeScreenshots: true,
        },
      }

      const screenshotResult = {
        screenshotPath: '/screenshots/task-123/iteration-1.png',
        thumbnailPath: '/screenshots/task-123/iteration-1-thumb.png',
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(repository)
      mockCapturePreviewScreenshot.mockResolvedValue(screenshotResult)

      await screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration)

      expect(mockGetTask).toHaveBeenCalledWith(taskId)
      expect(mockGetRepositoryById).toHaveBeenCalledWith('repo-123')
      expect(mockCapturePreviewScreenshot).toHaveBeenCalledWith(taskId, 1)
    })

    it('should update iteration with screenshot paths', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const repository = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        createdAt: new Date(),
        updatedAt: new Date(),
        codeExecution: { type: 'local' as const },
        previewConfig: {
          takeScreenshots: true,
        },
      }

      const screenshotResult = {
        screenshotPath: '/screenshots/task-123/iteration-1.png',
        thumbnailPath: '/screenshots/task-123/iteration-1-thumb.png',
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(repository)
      mockCapturePreviewScreenshot.mockResolvedValue(screenshotResult)

      await screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration)

      expect(iteration.screenshotPath).toBe('/screenshots/task-123/iteration-1.png')
      expect(iteration.thumbnailPath).toBe('/screenshots/task-123/iteration-1-thumb.png')
      expect(mockSaveTask).toHaveBeenCalledWith(task)
    })

    it('should emit event after screenshot capture', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const repository = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        createdAt: new Date(),
        updatedAt: new Date(),
        codeExecution: { type: 'local' as const },
        previewConfig: {
          takeScreenshots: true,
        },
      }

      const screenshotResult = {
        screenshotPath: '/screenshots/task-123/iteration-1.png',
        thumbnailPath: '/screenshots/task-123/iteration-1-thumb.png',
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(repository)
      mockCapturePreviewScreenshot.mockResolvedValue(screenshotResult)

      await screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration)

      expect(mockEmitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId,
          type: 'taskIteration',
          subType: 'updated',
          agentType: 'claude-code',
          iterationId: 1,
          payload: expect.objectContaining({
            status: 'completed',
            iterationNumber: 1,
            screenshotPath: '/screenshots/task-123/iteration-1.png',
            thumbnailPath: '/screenshots/task-123/iteration-1-thumb.png',
          }),
        }),
      )
    })

    it('should not throw if workspace service returns undefined', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const repository = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        createdAt: new Date(),
        updatedAt: new Date(),
        codeExecution: { type: 'local' as const },
        previewConfig: {
          takeScreenshots: true,
        },
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(repository)
      mockCapturePreviewScreenshot.mockResolvedValue(undefined)

      // Should not throw
      await expect(
        screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration),
      ).resolves.toBeUndefined()

      expect(mockSaveTask).not.toHaveBeenCalled()
      expect(mockEmitEvent).not.toHaveBeenCalled()
    })

    it('should handle errors gracefully without throwing', async () => {
      mockGetTask.mockRejectedValue(new Error('Storage error'))

      // Should not throw even if storage fails
      await expect(
        screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration),
      ).resolves.toBeUndefined()
    })

    it('should handle workspace service errors gracefully', async () => {
      const task: Task = {
        taskId,
        repositoryId: 'repo-123',
        agentType: 'claude-code',
        status: 'open',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const repository = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        createdAt: new Date(),
        updatedAt: new Date(),
        codeExecution: { type: 'local' as const },
        previewConfig: {
          takeScreenshots: true,
        },
      }

      mockGetTask.mockResolvedValue(task)
      mockGetRepositoryById.mockResolvedValue(repository)
      mockCapturePreviewScreenshot.mockRejectedValue(new Error('Screenshot failed'))

      // Should not throw even if screenshot fails
      await expect(
        screenshotService.captureIterationScreenshotIfEnabled(taskId, iteration),
      ).resolves.toBeUndefined()
    })
  })

  describe('captureScreenshot', () => {
    it('should be a method on the service', () => {
      expect(typeof screenshotService.captureScreenshot).toBe('function')
    })

    // Note: Full testing of captureScreenshot requires mocking Playwright browser
    // which is better suited for integration tests. We verify the method exists
    // and can be called in unit tests.
  })

  describe('captureSimpleScreenshot', () => {
    it('should be a method on the service', () => {
      expect(typeof screenshotService.captureSimpleScreenshot).toBe('function')
    })

    // Note: Full testing of captureSimpleScreenshot requires mocking Playwright browser
    // which is better suited for integration tests. We verify the method exists
    // and can be called with path parameter in unit tests.
  })

  describe('ensureBrowser', () => {
    it('should be a method on the service', () => {
      expect(typeof screenshotService.ensureBrowser).toBe('function')
    })

    // Note: Browser initialization requires Playwright mocking
    // which is better suited for integration tests
  })
})
