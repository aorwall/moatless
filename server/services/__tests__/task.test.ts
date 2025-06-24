import { beforeEach, describe, expect, it, vi } from 'vitest'
import { TaskNotFoundError, ValidationError } from '../../../shared/constants/errors.js'
import { EventType } from '../../../shared/types/event.js'
import type { CreateTaskRequest, TaskMetadata } from '../../../shared/types/task.js'
import type { EnvironmentService } from '../environment.js'
import type { EventService } from '../event.js'
import type { StorageBackend } from '../storage.js'
import { TaskService } from '../task.js'
import type { WorkspaceService } from '../workspace.js'

describe('TaskService', () => {
  let taskService: TaskService
  let mockStorage: StorageBackend
  let mockEventService: EventService
  let mockEnvironmentService: EnvironmentService
  let mockWorkspaceService: WorkspaceService

  beforeEach(() => {
    // Create mock services
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      list: vi.fn().mockResolvedValue([]),
      delete: vi.fn(),
      exists: vi.fn(),
      saveTask: vi.fn(),
      getTask: vi.fn(),
      saveEnvironment: vi.fn(),
      getEnvironment: vi.fn(),
      saveWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      saveEvents: vi.fn(),
      getEvents: vi.fn().mockResolvedValue([]),
    }

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
      createWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      updateWorkspace: vi.fn(),
      cleanupWorkspace: vi.fn(),
    } as any

    taskService = new TaskService(
      mockStorage,
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

      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
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

      // Verify start event was emitted
      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: EventType.START,
          taskId,
          isError: false,
        }),
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

      // Error event should be emitted
      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: EventType.ERROR,
          taskId,
          isError: true,
          payload: expect.objectContaining({
            error: 'Workspace setup failed',
            phase: 'workspace_setup',
          }),
        }),
      )
    })
  })

  describe('getTask', () => {
    it('should retrieve a task by ID', async () => {
      const task: TaskMetadata = {
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
      const tasks: TaskMetadata[] = Array.from({ length: 10 }, (_, i) => ({
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

      tasks.forEach((task) => {
        vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      })

      const result = await taskService.listTasks(5, 0)

      expect(result).toHaveLength(5)
      expect(result[0].taskId).toBe('task-0') // Most recent first
    })
  })

  describe('sendMessage', () => {
    it('should send a message to an active task', async () => {
      const task: TaskMetadata = {
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

      // Verify input event was emitted
      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: EventType.INPUT,
          taskId: 'task-123',
          iterationNumber: 1,
          payload: { message: 'Hello, task!' },
        }),
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
      const task: TaskMetadata = {
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
      const task: TaskMetadata = {
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
        () => new Promise((resolve) => setTimeout(() => resolve({ id: 'ws-123' } as any), 50)),
      )

      await taskService.createTask(request)

      // Background task should be running
      await taskService.waitForBackgroundTasks()

      // Workspace creation should have been called
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalled()
    })
  })
})
