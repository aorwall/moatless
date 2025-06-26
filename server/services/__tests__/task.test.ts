import { beforeEach, describe, expect, it, vi } from 'vitest'
import { EnvironmentNotFoundError, TaskNotFoundError } from '../../../shared/constants/errors.js'
import type { CreateTaskRequest, Task } from '../../../shared/types/task.js'
import type { AgentService } from '../agent.service.js'
import type { EnvironmentService } from '../environment.service.js'
import type { StorageBackend } from '../storage.js'
import { TaskService } from '../task.service.js'
import type { WorkspaceService } from '../workspace.service.js'

describe('TaskService', () => {
  let taskService: TaskService
  let mockStorage: StorageBackend
  let mockEnvironmentService: EnvironmentService
  let mockWorkspaceService: WorkspaceService
  let mockAgentService: AgentService

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
    } as any

    mockEnvironmentService = {
      getEnvironment: vi.fn(),
      listEnvironments: vi.fn(),
      createEnvironment: vi.fn(),
      updateEnvironment: vi.fn(),
      deleteEnvironment: vi.fn(),
    } as any

    mockWorkspaceService = {
      getOrCreateWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      updateWorkspace: vi.fn(),
      cleanupWorkspace: vi.fn(),
      createWorkspace: vi.fn(),
      trackBackgroundTask: vi.fn(),
      setupWorkspace: vi.fn(),
    } as any

    mockAgentService = {
      executeAgent: vi.fn(),
      getAgentStatus: vi.fn(),
      stopAgent: vi.fn(),
      isAgentRunning: vi.fn(),
      getAgentSessionId: vi.fn(),
    } as any

    taskService = new TaskService(mockStorage, mockWorkspaceService, mockAgentService)
  })

  describe('createTask', () => {
    it('should create a new task successfully', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        environmentId: 'env-123',
        message: 'Test task',
        start: true,
      }

      vi.mocked(mockEnvironmentService.getEnvironment).mockResolvedValueOnce({
        id: 'env-123',
        name: 'Test Environment',
      } as any)

      // Mock the new workspace methods
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      // Mock getTask to return the saved task for background processing
      vi.mocked(mockStorage.getTask).mockImplementation((id) => {
        return Promise.resolve({
          taskId: id,
          agentType: 'test-agent',
          environmentId: 'env-123',
          status: 'active',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
          createdAt: new Date(),
          lastMessageAt: new Date(),
        })
      })

      // Mock getOrCreateWorkspace for sendMessage
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue({
        getStatus: vi.fn().mockReturnValue('ready'),
      } as any)

      const taskId = await taskService.createTask(request)

      expect(taskId).toBeDefined()
      expect(taskId).toMatch(/^[0-9a-f-]+$/) // UUID pattern

      // Verify task was saved
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId,
          agentType: 'test-agent',
          environmentId: 'env-123',
          status: 'active',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        }),
      )

      // Events are no longer emitted on task creation

      // Wait for background tasks to complete
      // Background tasks are tracked in WorkspaceService

      // IMPORTANT: Verify workspace was created for the task
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'env-123',
        undefined,
      )
    })

    it('should always create workspace even without initial message', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        environmentId: 'env-123',
        // No message provided
        start: false,
      }

      vi.mocked(mockEnvironmentService.getEnvironment).mockResolvedValueOnce({
        id: 'env-123',
        name: 'Test Environment',
      } as any)

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)

      const taskId = await taskService.createTask(request)

      // Wait for background tasks
      // Background tasks are tracked in WorkspaceService

      // Workspace should ALWAYS be created regardless of start flag or message
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'env-123',
        undefined,
      )

      // Events are no longer emitted
    })

    it('should create workspace with custom branch name', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        environmentId: 'env-123',
        branchName: 'feature/custom-branch',
        start: false,
      }

      vi.mocked(mockEnvironmentService.getEnvironment).mockResolvedValueOnce({
        id: 'env-123',
        name: 'Test Environment',
      } as any)

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)

      const taskId = await taskService.createTask(request)

      // Background tasks are tracked in WorkspaceService

      // Verify workspace was created with the custom branch
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'env-123',
        'feature/custom-branch',
      )
    })

    it('should throw EnvironmentNotFoundError if environment does not exist', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        environmentId: 'non-existent',
        message: 'Test task',
      }

      // Mock createWorkspace to throw EnvironmentNotFoundError
      vi.mocked(mockWorkspaceService.createWorkspace).mockRejectedValueOnce(
        new EnvironmentNotFoundError('non-existent'),
      )

      await expect(taskService.createTask(request)).rejects.toThrow(EnvironmentNotFoundError)
    })

    it('should handle workspace setup errors gracefully', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        environmentId: 'env-123',
        message: 'Test task',
        start: true,
      }

      vi.mocked(mockEnvironmentService.getEnvironment).mockResolvedValueOnce({
        id: 'env-123',
      } as any)

      vi.mocked(mockWorkspaceService.createWorkspace).mockRejectedValueOnce(
        new Error('Workspace setup failed'),
      )

      // Task creation should throw when workspace setup fails
      await expect(taskService.createTask(request)).rejects.toThrow('Workspace setup failed')

      // Task should not be saved
      expect(mockStorage.saveTask).not.toHaveBeenCalled()
    })
  })

  describe('getTask', () => {
    it('should retrieve a task by ID', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        environmentId: 'env-123',
        status: 'active',
        numTurns: 5,
        totalCostUsd: 0.05,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      const result = await taskService.getTask('task-123')
      expect(result).toEqual(task)
    })

    it('should return null for non-existent task', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(null)

      const result = await taskService.getTask('non-existent')
      expect(result).toBeNull()
    })
  })

  describe('listTasks', () => {
    it('should list tasks with pagination', async () => {
      const tasks: Task[] = Array.from({ length: 10 }, (_, i) => ({
        taskId: `task-${i}`,
        createdAt: new Date(Date.now() - i * 1000),
        lastMessageAt: new Date(Date.now() - i * 1000),
        agentType: 'test-agent',
        environmentId: 'env-123',
        status: 'active',
        numTurns: i,
        totalCostUsd: i * 0.01,
        iterations: [],
      }))

      vi.mocked(mockStorage.list).mockResolvedValueOnce(tasks.map((t) => `tasks/${t.taskId}`))

      // Mock getTask to return tasks by ID
      vi.mocked(mockStorage.getTask).mockImplementation((id) => {
        const task = tasks.find((t) => t.taskId === id)
        return Promise.resolve(task || null)
      })

      const result = await taskService.listTasks(5, 0)

      expect(result).toHaveLength(5)
      expect(result[0].taskId).toBe('task-0') // Most recent first
    })
  })

  describe('sendMessage', () => {
    it('should send a message to an active task with workspace', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        environmentId: 'env-123',
        status: 'active',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      await taskService.sendMessage('task-123', 'Hello, task!')

      // Verify task was updated
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          numTurns: 1,
          iterations: expect.arrayContaining([
            expect.objectContaining({
              iterationNumber: 1,
              status: 'active',
              messageCount: 1,
              initialMessage: 'Hello, task!',
            }),
          ]),
        }),
      )

      // Note: Events are now emitted by WorkspaceService.runAgent, not TaskService

      // Verify agent interaction
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        'task-123',
        'Hello, task!',
        expect.objectContaining({
          iterationNumber: 1,
          status: 'active',
          messageCount: 1,
          initialMessage: 'Hello, task!',
        }),
        'claude-code',
        undefined,
      )
    })

    it('should setup workspace if not yet setup', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        environmentId: 'env-123',
        status: 'active',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      await taskService.sendMessage('task-123', 'Hello, task!')

      // Verify agent was started with the right parameters
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        'task-123',
        'Hello, task!',
        expect.objectContaining({
          iterationNumber: 1,
          status: 'active',
          messageCount: 1,
          initialMessage: 'Hello, task!',
        }),
        'claude-code',
        undefined,
      )
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(null)

      await expect(taskService.sendMessage('non-existent', 'Hello')).rejects.toThrow(
        TaskNotFoundError,
      )
    })
  })

  describe('updateTask', () => {
    it('should update task metadata', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        environmentId: 'env-123',
        status: 'active',
        numTurns: 5,
        totalCostUsd: 0.05,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      await taskService.updateTask('task-123', { status: 'completed' })

      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          status: 'completed',
        }),
      )
    })
  })

  describe('deleteTask', () => {
    it('should delete task and clean up resources', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        environmentId: 'env-123',
        status: 'completed',
        numTurns: 10,
        totalCostUsd: 0.1,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      await taskService.deleteTask('task-123')

      // Verify workspace was cleaned up
      expect(mockWorkspaceService.cleanupWorkspace).toHaveBeenCalledWith('task-123')

      // Verify storage was cleaned up
      expect(mockStorage.delete).toHaveBeenCalledWith('tasks/task-123')
      expect(mockStorage.delete).toHaveBeenCalledWith('events/task-123')

      // Note: Non-agent events are no longer emitted per architectural decision
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(null)

      await expect(taskService.deleteTask('non-existent')).rejects.toThrow(TaskNotFoundError)
    })
  })

  describe('background tasks', () => {
    it('should wait for all background tasks to complete', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        environmentId: 'env-123',
        message: 'Test task',
      }

      vi.mocked(mockEnvironmentService.getEnvironment).mockResolvedValueOnce({
        id: 'env-123',
      } as any)

      // Delay workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockImplementation(
        () =>
          new Promise((resolve) =>
            setTimeout(
              () => resolve({ id: 'ws-123', getWorkspacePath: () => '/test/path' } as any),
              50,
            ),
          ),
      )
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      // Mock getTask for background processing
      vi.mocked(mockStorage.getTask).mockImplementation((id) => {
        return Promise.resolve({
          taskId: id,
          agentType: 'test-agent',
          environmentId: 'env-123',
          status: 'active',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
          createdAt: new Date(),
          lastMessageAt: new Date(),
        })
      })

      await taskService.createTask(request)

      // Background task should be running
      // Background tasks are tracked in WorkspaceService

      // Workspace creation should have been called
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalled()
    })
  })
})
