import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../../../test/helper'

describe('Task Routes', () => {
  let app: Awaited<ReturnType<typeof build>>

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    // Hono doesn't need cleanup
  })

  describe('POST /api/tasks', () => {
    it('should create a new task with workspace', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const mockTask = {
        taskId: 'mock-task-id',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        repositoryId: 'test-env',
        numTurns: 0,
        totalCostUsd: 0,
        visibility: 'private' as const,
      }

      // Mock task creation to return task object
      vi.spyOn(taskFacade, 'createTask').mockResolvedValueOnce(mockTask)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            agentType: 'claude-code',
            repositoryId: 'test-env',
            message: 'Hello world',
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.taskId).toBe(mockTask.taskId)
      expect(body.status).toBe('open')

      // Verify task was created with correct params
      expect(taskFacade.createTask).toHaveBeenCalledWith({
        agentType: 'claude-code',
        repositoryId: 'test-env',
        message: 'Hello world',
      })
    })

    it('should always create workspace even without initial message', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const mockTask = {
        taskId: 'mock-task-id-2',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        repositoryId: 'test-env',
        numTurns: 0,
        totalCostUsd: 0,

        visibility: 'private' as const,
      }

      // Mock task creation to verify it's called with start: false
      vi.spyOn(taskFacade, 'createTask').mockResolvedValueOnce(mockTask)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            agentType: 'claude-code',
            repositoryId: 'test-env',
            message: 'Test message', // Required field
            start: false,
          }),
        }),
      )

      expect(response.status).toBe(200)

      // Verify the task was created with start: false
      expect(taskFacade.createTask).toHaveBeenCalledWith({
        agentType: 'claude-code',
        repositoryId: 'test-env',
        message: 'Test message',
        start: false,
      })
    })

    it('should return running status when task starts immediately', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const mockTask = {
        taskId: 'mock-task-id-running',
        status: 'running' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        repositoryId: 'test-env',
        numTurns: 1,
        totalCostUsd: 0,
        visibility: 'private' as const,
      }

      // Mock task creation
      vi.spyOn(taskFacade, 'createTask').mockResolvedValueOnce(mockTask)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            agentType: 'claude-code',
            repositoryId: 'test-env',
            message: 'Hello world',
            start: true,
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.taskId).toBe(mockTask.taskId)
      expect(body.status).toBe('running')
    })

    it('should validate request body', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            // Missing required fields
          }),
        }),
      )

      expect(response.status).toBe(400)
    })

    it('should return 400 if environment does not exist', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      // Import the error class
      const { EnvironmentNotFoundError } = await import('@moatless/types')

      // Mock task creation to throw environment not found error
      vi.spyOn(taskFacade, 'createTask').mockRejectedValueOnce(
        new EnvironmentNotFoundError('non-existent'),
      )

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            agentType: 'claude-code',
            repositoryId: 'non-existent',
            message: 'Hello world',
          }),
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('ENVIRONMENT_NOT_FOUND')
    })
  })

  describe('GET /api/tasks/:taskId', () => {
    it('should return task details', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const mockTask = {
        taskId: 'test-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        repositoryId: 'test-env',
        status: 'open' as const,
        numTurns: 0,
        totalCostUsd: 0,

        visibility: 'private' as const,
      }

      vi.spyOn(taskFacade, 'getTask').mockResolvedValueOnce(mockTask)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      const task = (await response.json()) as any
      expect(task.taskId).toBe('test-123')
      expect(task.agentType).toBe('claude-code')
    })

    it('should return 404 for non-existent task', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const { TaskNotFoundError } = await import('@moatless/types')
      const taskFacade = appContainer.get(TaskService)

      vi.spyOn(taskFacade, 'getTask').mockRejectedValueOnce(
        new TaskNotFoundError('non-existent-id'),
      )

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/non-existent-id', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('TASK_NOT_FOUND')
    })
  })

  describe('GET /api/tasks', () => {
    it('should list tasks with pagination', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const mockTasks = [
        {
          taskId: 'task-1',
          createdAt: new Date(),
          lastMessageAt: new Date(),
          agentType: 'claude-code',
          repositoryId: 'test-env',
          status: 'open' as const,
          numTurns: 0,
          totalCostUsd: 0,

          visibility: 'private' as const,
        },
        {
          taskId: 'task-2',
          createdAt: new Date(),
          lastMessageAt: new Date(),
          agentType: 'claude-code',
          repositoryId: 'test-env',
          status: 'closed' as const,
          numTurns: 5,
          totalCostUsd: 0.1,

          visibility: 'private' as const,
        },
      ]

      vi.spyOn(taskFacade, 'listTasks').mockResolvedValueOnce({
        tasks: mockTasks,
        total: mockTasks.length,
      })

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks?limit=10&offset=0', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.items).toHaveLength(2)
      expect(body.pagination).toBeDefined()
      expect(body.pagination.limit).toBe(10)
      expect(body.pagination.offset).toBe(0)
    })

    it('should filter tasks by status', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const mockTasks = [
        {
          taskId: 'task-running',
          status: 'running' as const,
          createdAt: new Date(),
          lastMessageAt: new Date(),
          agentType: 'claude-code',
          repositoryId: 'test-env',
          numTurns: 1,
          totalCostUsd: 0,
          visibility: 'private' as const,
        },
      ]

      vi.spyOn(taskFacade, 'listTasks').mockResolvedValueOnce({
        tasks: mockTasks,
        total: mockTasks.length,
      })

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks?status=running', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.items).toHaveLength(1)
      expect(body.items[0].status).toBe('running')
    })
  })

  describe('POST /api/tasks/:taskId/messages', () => {
    it('should send a message to task', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      vi.spyOn(taskFacade, 'sendMessage').mockResolvedValueOnce()

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123/messages', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            message: 'Test message',
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.status).toBe('sent')
      expect(body.timestamp).toBeDefined()
    })

    it('should validate message body', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123/messages', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            message: '', // Empty message
          }),
        }),
      )

      expect(response.status).toBe(400)
    })
  })

  describe('DELETE /api/tasks/:taskId', () => {
    it('should delete a task', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      vi.spyOn(taskFacade, 'deleteTask').mockResolvedValueOnce()

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123', {
          method: 'DELETE',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.status).toBe('deleted')
    })

    it('should return 404 if task not found', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      // Import the error class
      const { TaskNotFoundError } = await import('@moatless/types')

      vi.spyOn(taskFacade, 'deleteTask').mockRejectedValueOnce(
        new TaskNotFoundError('non-existent'),
      )

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/non-existent', {
          method: 'DELETE',
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('TASK_NOT_FOUND')
    })
  })

  describe('POST /api/tasks/:taskId/start', () => {
    it('should start a task', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      vi.spyOn(taskFacade, 'startTask').mockResolvedValueOnce()

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123/start', {
          method: 'POST',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.status).toBe('started')
      expect(body.taskId).toBe('test-123')

      expect(taskFacade.startTask).toHaveBeenCalledWith('test-123')
    })

    it('should return 400 when task cannot be started', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const { ValidationError } = await import('@moatless/types')

      vi.spyOn(taskFacade, 'startTask').mockRejectedValueOnce(
        new ValidationError(
          'Cannot start task with status "running". Only "open" or "stopped" tasks can be started.',
          {
            status: 'running',
          },
        ),
      )

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123/start', {
          method: 'POST',
        }),
      )

      expect(response.status).toBe(400)
      const body = (await response.json()) as any
      expect(body.error.message).toBe(
        'Cannot start task with status "running". Only "open" or "stopped" tasks can be started.',
      )
    })

    it('should return 404 when task not found', async () => {
      const { appContainer } = await import('../../../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskFacade = appContainer.get(TaskService)

      const { TaskNotFoundError } = await import('@moatless/types')

      vi.spyOn(taskFacade, 'startTask').mockRejectedValueOnce(new TaskNotFoundError('non-existent'))

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/non-existent/start', {
          method: 'POST',
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error.message).toContain('Task non-existent not found')
    })
  })
})
