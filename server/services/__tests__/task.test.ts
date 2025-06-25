import { beforeEach, describe, expect, it, vi } from 'vitest'
import { TaskNotFoundError, ValidationError } from '../../../shared/constants/errors.js'
import { EventType } from '../../../shared/types/event.js'
import type { CreateTaskRequest, Task } from '../../../shared/types/task.js'
import type { EnvironmentService } from '../environment.service.js'
import type { EventService } from '../event.service.js'
import { TaskService } from '../task.service.js'
import type { TaskStorageService } from '../task-storage.service.js'
import type { WorkspaceService } from '../workspace.service.js'

describe('TaskService', () => {
  let taskService: TaskService
  let mockTaskStorage: TaskStorageService
  let mockEventService: EventService
  let mockEnvironmentService: EnvironmentService
  let mockWorkspaceService: WorkspaceService

  beforeEach(() => {
    // Create mock services
    mockTaskStorage = {
      saveTask: vi.fn(),
      getTask: vi.fn(),
      listTasks: vi.fn().mockResolvedValue([]),
      deleteTask: vi.fn(),
      deleteTaskEvents: vi.fn(),
      getTaskWithEnvironmentId: vi.fn(),
    } as any

    mockEventService = {
      emitEvent: vi.fn(),
      getEvents: vi.fn(),
      subscribe: vi.fn(),
      subscribeAll: vi.fn(),
      subscribeByType: vi.fn(),
      getEventCount: vi.fn(),
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
      setupWorkspace: vi.fn(),
      startWorkspace: vi.fn(),
      triggerAgent: vi.fn(),
    } as any

    taskService = new TaskService(
      mockTaskStorage,
      mockEventService,
      mockEnvironmentService,
      mockWorkspaceService,
    )
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
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)
      vi.mocked(mockWorkspaceService.startWorkspace).mockResolvedValueOnce(undefined)
      
      // Mock getTask to return the saved task for background processing
      vi.mocked(mockTaskStorage.getTask).mockImplementation((id) => {
        return Promise.resolve({
          taskId: id,
          agentType: 'test-agent',
          environmentId: 'env-123',
          status: 'active',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
          createdAt: new Date(),
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
      expect(mockTaskStorage.saveTask).toHaveBeenCalledWith(
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
      await taskService.waitForBackgroundTasks()

      // IMPORTANT: Verify workspace was created for the task
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(taskId, undefined)
      expect(mockWorkspaceService.setupWorkspace).toHaveBeenCalledWith(taskId)
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
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)

      const taskId = await taskService.createTask(request)

      // Wait for background tasks
      await taskService.waitForBackgroundTasks()

      // Workspace should ALWAYS be created regardless of start flag or message
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(taskId, undefined)

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
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)

      const taskId = await taskService.createTask(request)

      await taskService.waitForBackgroundTasks()

      // Verify workspace was created with the custom branch
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'feature/custom-branch',
      )
    })

    it('should throw ValidationError if environment does not exist', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        environmentId: 'non-existent',
        message: 'Test task',
      }

      vi.mocked(mockEnvironmentService.getEnvironment).mockResolvedValueOnce(null)

      await expect(taskService.createTask(request)).rejects.toThrow(ValidationError)
      await expect(taskService.createTask(request)).rejects.toThrow(
        'Environment non-existent not found',
      )
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

      const taskId = await taskService.createTask(request)

      // Task should still be created
      expect(taskId).toBeDefined()

      // Wait for background task to complete
      await taskService.waitForBackgroundTasks()

      // Error events are no longer emitted
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

      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(task)

      const result = await taskService.getTask('task-123')
      expect(result).toEqual(task)
    })

    it('should return null for non-existent task', async () => {
      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(null)

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

      vi.mocked(mockTaskStorage.listTasks).mockResolvedValueOnce(
        tasks.map((t) => `tasks/${t.taskId}`),
      )

      tasks.forEach((task) => {
        vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(task)
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

      const mockWorkspace = {
        getStatus: vi.fn().mockReturnValue('setup_completed'),
        setup: vi.fn(),
        triggerAgent: vi.fn(),
      }

      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValueOnce(
        mockWorkspace as any,
      )
      vi.mocked(mockWorkspaceService.startWorkspace).mockResolvedValueOnce(undefined)

      await taskService.sendMessage('task-123', 'Hello, task!')

      // Verify task was updated
      expect(mockTaskStorage.saveTask).toHaveBeenCalledWith(
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

      // Verify input event was emitted
      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: EventType.INPUT,
          taskId: 'task-123',
          iterationId: 1,
          payload: { message: 'Hello, task!' },
        }),
      )

      // Verify workspace interaction
      expect(mockWorkspaceService.getOrCreateWorkspace).toHaveBeenCalledWith('task-123')
      expect(mockWorkspaceService.startWorkspace).toHaveBeenCalledWith('task-123', 'test-agent', 'Hello, task!', 1)
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

      const mockWorkspace = {
        getStatus: vi.fn().mockReturnValue('not_setup'),
        setup: vi.fn(),
        triggerAgent: vi.fn(),
      }

      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValueOnce(
        mockWorkspace as any,
      )
      vi.mocked(mockWorkspaceService.startWorkspace).mockResolvedValueOnce(undefined)

      await taskService.sendMessage('task-123', 'Hello, task!')

      // Verify workspace was started with the right parameters
      expect(mockWorkspaceService.startWorkspace).toHaveBeenCalledWith(
        'task-123', 
        'test-agent', 
        'Hello, task!', 
        1
      )
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(null)

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

      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(task)

      await taskService.updateTask('task-123', { status: 'completed' })

      expect(mockTaskStorage.saveTask).toHaveBeenCalledWith(
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

      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(task)

      await taskService.deleteTask('task-123')

      // Verify workspace was cleaned up
      expect(mockWorkspaceService.cleanupWorkspace).toHaveBeenCalledWith('task-123')

      // Verify storage was cleaned up
      expect(mockTaskStorage.deleteTask).toHaveBeenCalledWith('task-123')
      expect(mockTaskStorage.deleteTaskEvents).toHaveBeenCalledWith('task-123')

      // Verify stop event was emitted
      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: EventType.STOP,
          taskId: 'task-123',
          payload: { reason: 'deleted' },
        }),
      )
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockTaskStorage.getTask).mockResolvedValueOnce(null)

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
        () => new Promise((resolve) => setTimeout(() => resolve({ id: 'ws-123', getWorkspacePath: () => '/test/path' } as any), 50)),
      )
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)
      vi.mocked(mockWorkspaceService.startWorkspace).mockResolvedValueOnce(undefined)
      
      // Mock getTask for background processing
      vi.mocked(mockTaskStorage.getTask).mockImplementation((id) => {
        return Promise.resolve({
          taskId: id,
          agentType: 'test-agent',
          environmentId: 'env-123',
          status: 'active',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
          createdAt: new Date(),
        })
      })
      
      // Mock getOrCreateWorkspace for sendMessage
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue({
        getStatus: vi.fn().mockReturnValue('ready'),
      } as any)

      await taskService.createTask(request)

      // Background task should be running
      await taskService.waitForBackgroundTasks()

      // Workspace creation should have been called
      expect(mockWorkspaceService.getOrCreateWorkspace).toHaveBeenCalled()
    })
  })
})
