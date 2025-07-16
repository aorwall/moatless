import type { RepositoryConfig, Task } from '@moatless/types'
import { WorkspaceStatus } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { EventService } from '../event.service.js'
import type { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'
import { WorkspaceService } from '../workspace.service.js'

// Mock fs promises
vi.mock('fs', () => ({
  promises: {
    access: vi.fn(() => Promise.resolve()),
    mkdir: vi.fn(() => Promise.resolve()),
    writeFile: vi.fn(() => Promise.resolve()),
    readFile: vi.fn(() => Promise.resolve('file content')),
    rm: vi.fn(() => Promise.resolve()),
    stat: vi.fn(() => Promise.resolve({ isFile: () => false, isDirectory: () => true })),
    readdir: vi.fn(() => Promise.resolve([])),
  },
}))

// Mock child_process
vi.mock('child_process', () => ({
  exec: vi.fn((_cmd, opts, cb) => {
    if (cb) {
      cb(null, { stdout: 'mocked output', stderr: '' })
    } else if (typeof opts === 'function') {
      opts(null, { stdout: 'mocked output', stderr: '' })
    }
  }),
}))

// Mock git-utils
vi.mock('../../utils/git-utils.js', () => ({
  getCurrentBranch: vi.fn().mockResolvedValue('main'),
  createBranch: vi.fn().mockResolvedValue(undefined),
  getGitDiffFiles: vi.fn().mockResolvedValue({
    files: [],
    totalChanges: 0,
    totalAdditions: 0,
    totalDeletions: 0,
  }),
  isGitRepository: vi.fn().mockResolvedValue(true),
  hasUncommittedChanges: vi.fn().mockResolvedValue(false),
  getGitDiffString: vi.fn().mockResolvedValue(['', '']),
}))

describe('WorkspaceService Status Management', () => {
  let workspaceService: WorkspaceService
  let mockRepositoryService: RepositoryService
  let mockEventService: EventService
  let mockStorage: StorageBackend

  const mockTask: Task = {
    taskId: 'test-task-123',
    repositoryId: 'env-123',
    agentType: 'test-agent',
    status: 'open',
    createdAt: new Date(),
    lastMessageAt: new Date(),
    numTurns: 0,
    totalCostUsd: 0,
    iterations: [],
  }

  const mockRepository: RepositoryConfig = {
    id: 'env-123',
    name: 'Test Repository',
    description: 'Test repository',
    path: '/test/repo',
    useGitWorktrees: true,
    codeExecution: {
      type: 'local',
    },
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  beforeEach(() => {
    vi.clearAllMocks()

    mockRepositoryService = {
      getRepositoryById: vi.fn().mockResolvedValue(mockRepository),
    } as unknown as RepositoryService

    mockEventService = {
      emit: vi.fn(),
      emitEvent: vi.fn(),
    } as unknown as EventService

    mockStorage = {
      getTask: vi.fn().mockResolvedValue(mockTask),
      saveTask: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      read: vi.fn(),
      write: vi.fn(),
      list: vi.fn(),
    } as unknown as StorageBackend

    workspaceService = new WorkspaceService(mockRepositoryService, mockEventService, mockStorage)
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
      // Use a unique task ID for this test
      const uniqueTaskId = 'test-task-existing'
      vi.mocked(mockStorage.getTask).mockResolvedValue({ ...mockTask, taskId: uniqueTaskId })

      const workspace1 = await workspaceService.getOrCreateWorkspace(uniqueTaskId)

      // Clear mock call count after first workspace creation
      vi.mocked(mockRepositoryService.getRepositoryById).mockClear()

      const workspace2 = await workspaceService.getOrCreateWorkspace(uniqueTaskId)

      expect(workspace1).toBe(workspace2)
      // Should not call getRepositoryById again for existing workspace
      expect(mockRepositoryService.getRepositoryById).not.toHaveBeenCalled()
    })

    it('should throw error if task not found', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(null)

      await expect(workspaceService.getOrCreateWorkspace('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )
    })

    it('should throw error if repository not found', async () => {
      vi.mocked(mockRepositoryService.getRepositoryById).mockResolvedValue(null)

      await expect(workspaceService.getOrCreateWorkspace('test-task-123')).rejects.toThrow(
        'Repository env-123 not found',
      )
    })
  })

  describe('workspace status checking', () => {
    it('should prevent operations on workspace with NOT_SETUP status', async () => {
      const taskId = 'test-task-status'
      vi.mocked(mockStorage.getTask).mockResolvedValue({ ...mockTask, taskId })

      const mockWorkspace = {
        id: taskId,
        type: 'local',
        getStatus: vi.fn().mockReturnValue(WorkspaceStatus.NOT_SETUP),
        listFiles: vi.fn().mockResolvedValue(['file1.txt', 'file2.txt']),
        readFile: vi.fn().mockResolvedValue('content'),
        writeFile: vi.fn().mockResolvedValue(undefined),
        getGitDiff: vi.fn().mockResolvedValue('diff'),
        hasUncommittedChanges: vi.fn().mockResolvedValue(false),
      } as unknown

      // Create workspace
      await workspaceService.getOrCreateWorkspace(taskId)

      // Override the workspace with our mock

      ;(
        workspaceService as unknown as { activeWorkspaces: Map<string, unknown> }
      ).activeWorkspaces.set(taskId, mockWorkspace)

      // These operations should fail with NOT_SETUP status
      await expect(workspaceService.listFiles(taskId)).rejects.toThrow(
        'Cannot list files: workspace not ready',
      )
      await expect(workspaceService.readFile(taskId, 'file1.txt')).rejects.toThrow(
        'Cannot read file: workspace not ready',
      )
      await expect(workspaceService.writeFile(taskId, 'file1.txt', 'new content')).rejects.toThrow(
        'Cannot write file: workspace not ready',
      )
      await expect(workspaceService.getGitDiff(taskId)).rejects.toThrow(
        'Cannot access git: workspace not ready',
      )

      const status = await workspaceService.getWorkspaceStatus(taskId)
      expect((status as any).status).toBe(WorkspaceStatus.NOT_SETUP)
      expect((status as any).hasUncommittedChanges).toBe(false)
    })
  })

  describe('workspace operations without existing task', () => {
    it('should throw error when task not found for file operations', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(null)

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
      vi.mocked(mockStorage.getTask).mockResolvedValue(null)

      await expect(workspaceService.getGitDiff('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )

      await expect(workspaceService.getWorkspaceStatus('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )
    })

    it('should throw error when task not found for agent operations', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(null)

      await expect(workspaceService.setupWorkspace('non-existent')).rejects.toThrow(
        'Task non-existent not found',
      )
    })

    it('should return gracefully for cleanup when workspace not found', async () => {
      // Cleanup uses activeWorkspaces.get directly, so it doesn't throw when workspace doesn't exist
      await expect(workspaceService.cleanupWorkspace('non-existent')).resolves.toBeUndefined()
    })
  })
})
