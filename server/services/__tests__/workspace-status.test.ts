import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Environment } from '../../../shared/types/environment.js'
import type { Task } from '../../../shared/types/task.js'
import { WorkspaceStatus } from '../../../shared/types/workspace.js'
import type { EnvironmentService } from '../environment.service.js'
import type { EventService } from '../event.service.js'
import type { TaskStorageService } from '../task-storage.service.js'
import { WorkspaceService } from '../workspace.service.js'

describe('WorkspaceService Status Management', () => {
  let workspaceService: WorkspaceService
  let mockEnvironmentService: EnvironmentService
  let mockEventService: EventService
  let mockTaskStorageService: TaskStorageService

  const mockTask: Task = {
    taskId: 'test-task-123',
    environmentId: 'env-123',
    agentType: 'test-agent',
    status: 'active',
    createdAt: new Date(),
    lastMessageAt: new Date(),
    numTurns: 0,
    totalCostUsd: 0,
    iterations: [],
  }

  const mockEnvironment: Environment = {
    id: 'env-123',
    name: 'Test Environment',
    description: 'Test environment',
    repository: {
      type: 'local',
      path: '/test/repo',
      useGitWorktrees: true,
    },
    codeExecution: {
      type: 'local',
    },
    variables: {},
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  beforeEach(() => {
    mockEnvironmentService = {
      getEnvironment: vi.fn().mockResolvedValue(mockEnvironment),
    } as any

    mockEventService = {
      emit: vi.fn(),
      emitEvent: vi.fn(),
    } as any

    mockTaskStorageService = {
      getTaskWithEnvironmentId: vi.fn().mockResolvedValue({
        task: mockTask,
        environmentId: mockTask.environmentId,
      }),
    } as any

    workspaceService = new WorkspaceService(
      mockEnvironmentService,
      mockEventService,
      mockTaskStorageService,
    )
  })

  describe('getOrCreateWorkspace', () => {
    it('should create and store workspace immediately', async () => {
      const workspace = await workspaceService.getOrCreateWorkspace('test-task-123')

      expect(workspace).toBeDefined()
      expect(workspace.id).toBe('test-task-123')

      // Verify workspace is immediately available
      const retrieved = await workspaceService.getWorkspace('test-task-123')
      expect(retrieved).toBe(workspace)
    })

    it('should return existing workspace if already created', async () => {
      const workspace1 = await workspaceService.getOrCreateWorkspace('test-task-123')
      const workspace2 = await workspaceService.getOrCreateWorkspace('test-task-123')

      expect(workspace1).toBe(workspace2)
      expect(mockEnvironmentService.getEnvironment).toHaveBeenCalledTimes(1)
    })

    it('should throw error if task not found', async () => {
      vi.mocked(mockTaskStorageService.getTaskWithEnvironmentId).mockResolvedValue(null)

      await expect(workspaceService.getOrCreateWorkspace('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )
    })

    it('should throw error if environment not found', async () => {
      vi.mocked(mockEnvironmentService.getEnvironment).mockResolvedValue(null)

      await expect(workspaceService.getOrCreateWorkspace('test-task-123')).rejects.toThrow(
        'Environment env-123 not found',
      )
    })
  })

  describe('workspace status checking', () => {
    it('should allow operations on workspace regardless of status', async () => {
      const mockWorkspace = {
        id: 'test-task-123',
        type: 'local',
        getStatus: vi.fn().mockReturnValue(WorkspaceStatus.NOT_SETUP),
        listFiles: vi.fn().mockResolvedValue(['file1.txt', 'file2.txt']),
        readFile: vi.fn().mockResolvedValue('content'),
        writeFile: vi.fn().mockResolvedValue(undefined),
        getGitDiff: vi.fn().mockResolvedValue('diff'),
        hasUncommittedChanges: vi.fn().mockResolvedValue(false),
      } as any

      // Create workspace
      await workspaceService.getOrCreateWorkspace('test-task-123')

      // Override the workspace with our mock

      ;(workspaceService as any).activeWorkspaces.set('test-task-123', mockWorkspace)

      // These operations should work even with NOT_SETUP status
      await expect(workspaceService.listFiles('test-task-123')).resolves.toEqual([
        'file1.txt',
        'file2.txt',
      ])
      await expect(workspaceService.readFile('test-task-123', 'file1.txt')).resolves.toBe('content')
      await expect(
        workspaceService.writeFile('test-task-123', 'file1.txt', 'new content'),
      ).resolves.toBeUndefined()
      await expect(workspaceService.getGitDiff('test-task-123')).resolves.toBe('diff')

      const status = await workspaceService.getWorkspaceStatus('test-task-123')
      expect(status.status).toBe(WorkspaceStatus.NOT_SETUP)
      expect(status.hasUncommittedChanges).toBe(false)
    })
  })

  describe('workspace operations without existing task', () => {
    it('should throw error when task not found for file operations', async () => {
      vi.mocked(mockTaskStorageService.getTaskWithEnvironmentId).mockResolvedValue(null)

      await expect(workspaceService.listFiles('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )

      await expect(workspaceService.readFile('non-existent', 'file.txt')).rejects.toThrow(
        'Task non-existent not found',
      )

      await expect(
        workspaceService.writeFile('non-existent', 'file.txt', 'content'),
      ).rejects.toThrow('Task non-existent not found')
    })

    it('should throw error when task not found for git operations', async () => {
      vi.mocked(mockTaskStorageService.getTaskWithEnvironmentId).mockResolvedValue(null)

      await expect(workspaceService.getGitDiff('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )

      await expect(workspaceService.getWorkspaceStatus('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )
    })

    it('should throw error when task not found for agent operations', async () => {
      vi.mocked(mockTaskStorageService.getTaskWithEnvironmentId).mockResolvedValue(null)

      await expect(workspaceService.setupWorkspace('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )

      await expect(
        workspaceService.triggerAgent('non-existent', 'agent-type', 'message', 1),
      ).rejects.toThrow('Task non-existent not found')
    })

    it('should return gracefully for cleanup when workspace not found', async () => {
      // Cleanup uses activeWorkspaces.get directly, so it doesn't throw when workspace doesn't exist
      await expect(workspaceService.cleanupWorkspace('non-existent')).resolves.toBeUndefined()
    })
  })
})
