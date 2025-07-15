import type { CreateVibeTaskRequest } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AgentService } from '../agent.service.js'
import type { CommandService } from '../command.service.js'
import type { EventService } from '../event.service.js'
import type { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'
import { TaskService } from '../task.service.js'
import type { WorkspaceService } from '../workspace.service.js'

// Mock the file system operations
vi.mock('node:fs', () => ({
  promises: {
    mkdir: vi.fn().mockResolvedValue(undefined),
    access: vi.fn().mockResolvedValue(undefined),
  },
}))

// Mock child_process
vi.mock('node:child_process', () => ({
  exec: vi.fn(),
}))

describe('TaskService - createVibeTask', () => {
  let taskService: TaskService
  let mockStorage: StorageBackend
  let mockEventService: EventService
  let mockWorkspaceService: WorkspaceService
  let mockAgentService: AgentService
  let mockCommandService: CommandService
  let mockRepositoryService: RepositoryService

  beforeEach(() => {
    // Create mock services
    mockStorage = {
      saveTask: vi.fn(),
      getTask: vi.fn(),
      list: vi.fn().mockResolvedValue([]),
      delete: vi.fn(),
      exists: vi.fn(),
      read: vi.fn(),
      write: vi.fn(),
    } as unknown as StorageBackend

    mockEventService = {
      emitEvent: vi.fn().mockResolvedValue(undefined),
    } as unknown as EventService

    mockWorkspaceService = {
      createWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      setupWorkspace: vi.fn(),
      trackBackgroundTask: vi.fn(),
      stopServer: vi.fn(),
      cleanupWorkspace: vi.fn(),
    } as unknown as WorkspaceService

    mockAgentService = {
      executeAgent: vi.fn(),
      stopAgent: vi.fn(),
      getTaskRunningStatus: vi.fn(),
    } as unknown as AgentService

    mockCommandService = {
      getCommand: vi.fn(),
    } as unknown as CommandService

    mockRepositoryService = {
      getRepositoryById: vi.fn(),
      createRepository: vi.fn(),
      listRepositories: vi.fn(),
      updateRepository: vi.fn(),
    } as unknown as RepositoryService

    taskService = new TaskService(
      mockStorage,
      mockEventService,
      mockWorkspaceService,
      mockAgentService,
      mockCommandService,
      mockRepositoryService,
    )
  })

  describe('createVibeTask', () => {
    it('should create a vibe task from basic-hello-website template successfully', async () => {
      const request: CreateVibeTaskRequest = {
        agentType: 'test-agent',
        templateId: 'basic-hello-website',
        repositoryName: 'my-hello-website',
        message: 'Create a new website',
        start: false,
      }

      // Mock repository creation
      const mockRepository = {
        id: 'repo-123',
        name: 'my-hello-website',
        description: 'Created from template: Basic Hello Website',
        path: '/data/templates/basic-hello-website',
        useGitWorktrees: true,
        codeExecution: {
          type: 'local',
          setupCommands: 'npm install',
        },
        previewConfig: {
          serverStartCommand: 'npm run dev',
          injectionStrategy: {
            method: 'html-injection',
          },
        },
      }

      vi.mocked(mockRepositoryService.createRepository).mockResolvedValueOnce(mockRepository as any)

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)

      // Mock task retrieval for status
      vi.mocked(mockStorage.getTask).mockImplementation((id) => {
        return Promise.resolve({
          taskId: id,
          agentType: 'test-agent',
          repositoryId: 'repo-123',
          status: 'open',
          numTurns: 0,
          totalCostUsd: 0.0,
          iterations: [],
          createdAt: new Date(),
          lastMessageAt: new Date(),
          workspacePath: '/test/workspace',
        })
      })

      const taskId = await taskService.createVibeTask(request)

      expect(taskId).toBeDefined()
      expect(typeof taskId).toBe('string')

      // Verify repository creation was called with correct parameters
      expect(mockRepositoryService.createRepository).toHaveBeenCalledWith({
        name: 'my-hello-website',
        description: 'Created from template: Basic Hello Website',
        path: expect.stringContaining('basic-hello-website'),
        useGitWorktrees: true,
        codeExecution: {
          type: 'local',
          setupCommands: 'npm install',
        },
        previewConfig: {
          serverStartCommand: 'npm run dev',
          injectionStrategy: {
            method: 'html-injection',
          },
        },
      })

      // Verify workspace creation was called
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'repo-123',
        undefined,
      )

      // Verify task was saved
      expect(mockStorage.saveTask).toHaveBeenCalled()
    })

    it('should throw ValidationError for unknown template', async () => {
      const request: CreateVibeTaskRequest = {
        agentType: 'test-agent',
        templateId: 'unknown-template',
        repositoryName: 'my-website',
        start: false,
      }

      await expect(taskService.createVibeTask(request)).rejects.toThrow(
        'Template with id unknown-template not found',
      )

      // Verify no repository was created
      expect(mockRepositoryService.createRepository).not.toHaveBeenCalled()
    })

    it('should create repository with correct template configuration', async () => {
      const request: CreateVibeTaskRequest = {
        agentType: 'test-agent',
        templateId: 'basic-hello-website',
        repositoryName: 'my-custom-website',
        start: false,
      }

      // Mock repository creation
      vi.mocked(mockRepositoryService.createRepository).mockResolvedValueOnce({
        id: 'repo-456',
        name: 'my-custom-website',
      } as any)

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        getWorkspacePath: () => '/test/workspace',
      } as any)

      // Mock task retrieval
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce({
        taskId: 'test-task-id',
        status: 'open',
      } as any)

      await taskService.createVibeTask(request)

      // Verify repository creation was called with template configuration
      expect(mockRepositoryService.createRepository).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'my-custom-website',
          description: 'Created from template: Basic Hello Website',
          useGitWorktrees: true,
          codeExecution: expect.objectContaining({
            type: 'local',
            setupCommands: 'npm install',
          }),
          previewConfig: expect.objectContaining({
            serverStartCommand: 'npm run dev',
            }),
        }),
      )
    })

    it('should handle optional parameters correctly', async () => {
      const request: CreateVibeTaskRequest = {
        agentType: 'custom-agent',
        templateId: 'basic-hello-website',
        repositoryName: 'my-website',
        message: 'Custom message',
        start: false, // Set to false to avoid agent execution complexity
        model: 'gpt-4',
        filePaths: ['/path/to/file1.txt', '/path/to/file2.txt'],
      }

      // Mock repository creation
      vi.mocked(mockRepositoryService.createRepository).mockResolvedValueOnce({
        id: 'repo-123',
        name: 'my-website',
      } as any)

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        getWorkspacePath: () => '/test/workspace',
      } as any)

      // Mock task retrieval
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce({
        taskId: 'test-task-id',
        status: 'open',
      } as any)

      const taskId = await taskService.createVibeTask(request)

      expect(taskId).toBeDefined()

      // Verify repository was created
      expect(mockRepositoryService.createRepository).toHaveBeenCalled()

      // Verify workspace was created with correct parameters
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'repo-123',
        undefined,
      )
    })
  })
})
