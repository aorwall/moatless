import type { CreateTaskRequest } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StorageBackend } from '../../storage/index.js'
import type { CommandService } from '../command.service.js'
import type { EventService } from '../event.service.js'
import type { RepositoryService } from '../repository.service.js'
import { TaskService } from '../task.service.js'
import type { WorkspaceService } from '../workspace.service.js'

describe('TaskService newBranch Integration Tests', () => {
  let taskService: TaskService
  let mockStorage: StorageBackend
  let mockEventService: EventService
  let mockWorkspaceService: WorkspaceService
  let mockCommandService: CommandService
  let mockRepositoryService: RepositoryService

  beforeEach(() => {
    // Create mock instances
    mockStorage = {
      saveTask: vi.fn().mockResolvedValue(undefined),
      getTask: vi.fn(),
      list: vi.fn(),
      delete: vi.fn(),
      get: vi.fn(),
      set: vi.fn(),
    } as any

    mockEventService = {
      emitEvent: vi.fn().mockResolvedValue(undefined),
      subscribeAll: vi.fn(),
    } as any

    mockWorkspaceService = {
      createWorkspace: vi.fn().mockResolvedValue({
        getWorkspacePath: () => '/test/workspace/path',
      }),
      setupWorkspace: vi.fn().mockResolvedValue(undefined),
      trackBackgroundTask: vi.fn(),
    } as any

    mockCommandService = {} as any
    mockRepositoryService = {} as any

    const mockAgentService = {} as any
    const mockScreenshotService = {} as any

    // Create TaskService instance
    taskService = new TaskService(
      mockStorage,
      mockEventService,
      mockWorkspaceService,
      mockCommandService,
      mockRepositoryService,
      mockAgentService,
      mockScreenshotService,
    )
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('createTask with newBranch parameter', () => {
    it('should pass newBranch parameter to workspace creation', async () => {
      const createTaskParams: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        message: 'Test message',
        newBranch: 'feature/custom-branch',
        start: false, // Don't start to avoid agent complications
      }

      await taskService.createTask(createTaskParams)

      // Verify that createWorkspace was called with the newBranch parameter
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        expect.any(String), // taskId
        'test-repo-123',
        undefined, // branchName/originalBranch (not provided)
        'feature/custom-branch', // newBranch
      )
    })

    it('should pass both originalBranch and newBranch when both are provided', async () => {
      const createTaskParams: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        message: 'Test message',
        originalBranch: 'develop',
        newBranch: 'feature/from-develop',
        start: false,
      }

      await taskService.createTask(createTaskParams)

      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        expect.any(String), // taskId
        'test-repo-123',
        'develop', // originalBranch (source branch)
        'feature/from-develop', // newBranch (target branch)
      )
    })

    it('should fallback to branchName when originalBranch not provided', async () => {
      const createTaskParams: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        message: 'Test message',
        branchName: 'legacy-branch', // deprecated but still supported
        newBranch: 'feature/from-legacy',
        start: false,
      }

      await taskService.createTask(createTaskParams)

      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        expect.any(String), // taskId
        'test-repo-123',
        'legacy-branch', // branchName (fallback)
        'feature/from-legacy', // newBranch
      )
    })

    it('should work without newBranch for backward compatibility', async () => {
      const createTaskParams: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        message: 'Test message',
        originalBranch: 'main',
        start: false,
      }

      await taskService.createTask(createTaskParams)

      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        expect.any(String), // taskId
        'test-repo-123',
        'main', // originalBranch
        undefined, // newBranch (not provided)
      )
    })

    it('should handle minimal required parameters', async () => {
      const createTaskParams: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        // No message, no branches - minimal case
      }

      await taskService.createTask(createTaskParams)

      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        expect.any(String), // taskId
        'test-repo-123',
        undefined, // no branch specified
        undefined, // no newBranch specified
      )
    })

    it('should create task with proper metadata when newBranch is provided', async () => {
      const createTaskParams: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        message: 'Test message with custom branch',
        newBranch: 'feature/test-branch',
        start: false,
      }

      const task = await taskService.createTask(createTaskParams)

      // Verify task metadata
      expect(task).toMatchObject({
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0.0,
        workspacePath: '/test/workspace/path',
      })

      // Verify task was saved
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          agentType: 'claude-code',
          repositoryId: 'test-repo-123',
          workspacePath: '/test/workspace/path',
        }),
      )

      // Verify event was emitted
      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'task',
          subType: 'open',
          payload: {
            status: 'open',
            previousStatus: 'created',
          },
        }),
      )
    })

    it('should handle various branch name formats', async () => {
      const branchNames = [
        'feature/new-feature',
        'bugfix/fix-issue-123',
        'hotfix/critical',
        'release/v1.0.0',
        'user/john/feature',
        'develop',
        'main',
      ]

      for (const branchName of branchNames) {
        vi.clearAllMocks()

        const createTaskParams: CreateTaskRequest = {
          agentType: 'claude-code',
          repositoryId: 'test-repo-123',
          newBranch: branchName,
          start: false,
        }

        await taskService.createTask(createTaskParams)

        expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
          expect.any(String),
          'test-repo-123',
          undefined,
          branchName,
        )
      }
    })
  })

  describe('Error handling with newBranch', () => {
    it('should handle workspace creation failure gracefully', async () => {
      const createTaskParams: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        newBranch: 'feature/failing-branch',
        start: false,
      }

      // Mock workspace creation to fail
      mockWorkspaceService.createWorkspace = vi
        .fn()
        .mockRejectedValue(new Error('Branch creation failed'))

      await expect(taskService.createTask(createTaskParams)).rejects.toThrow(
        'Branch creation failed',
      )

      // Verify task metadata was still saved (before workspace creation)
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          agentType: 'claude-code',
          repositoryId: 'test-repo-123',
        }),
      )
    })
  })
})
