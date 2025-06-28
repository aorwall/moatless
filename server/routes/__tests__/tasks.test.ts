import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'

describe('Task Routes', () => {
  let app: FastifyInstance

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    await app.close()
  })

  describe('POST /api/tasks', () => {
    it('should create a new task with workspace', async () => {
      const mockTaskId = 'mock-task-id'

      // Mock task creation
      vi.spyOn(app.services.task, 'createTask').mockResolvedValueOnce(mockTaskId)

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/tasks',
        payload: {
          agentType: 'claude-code',
          environmentId: 'test-env',
          message: 'Hello world',
        },
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.taskId).toBe(mockTaskId)
      expect(body.status).toBe('open')

      // Verify task was created with correct params
      expect(app.services.task.createTask).toHaveBeenCalledWith({
        agentType: 'claude-code',
        environmentId: 'test-env',
        message: 'Hello world',
      })
    })

    it('should always create workspace even without initial message', async () => {
      const mockTaskId = 'mock-task-id-2'

      // Mock task creation to verify it's called with start: false
      vi.spyOn(app.services.task, 'createTask').mockResolvedValueOnce(mockTaskId)

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/tasks',
        payload: {
          agentType: 'claude-code',
          environmentId: 'test-env',
          message: 'Test message', // Required field
          start: false,
        },
      })

      expect(response.statusCode).toBe(200)

      // Verify the task was created with start: false
      expect(app.services.task.createTask).toHaveBeenCalledWith({
        agentType: 'claude-code',
        environmentId: 'test-env',
        message: 'Test message',
        start: false,
      })
    })

    it('should validate request body', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/tasks',
        payload: {
          // Missing required fields
        },
      })

      expect(response.statusCode).toBe(400)
      const body = JSON.parse(response.body)
      expect(body.error).toBeDefined()
      expect(body.error.code).toBe('VALIDATION_ERROR')
    })

    it('should return 400 if environment does not exist', async () => {
      // Import the error class
      const { EnvironmentNotFoundError } = await import('../../../shared/constants/errors.js')

      // Mock task creation to throw environment not found error
      vi.spyOn(app.services.task, 'createTask').mockRejectedValueOnce(
        new EnvironmentNotFoundError('non-existent'),
      )

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/tasks',
        payload: {
          agentType: 'claude-code',
          environmentId: 'non-existent',
          message: 'Hello world',
        },
      })

      expect(response.statusCode).toBe(404)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('ENVIRONMENT_NOT_FOUND')
    })
  })

  describe('GET /api/tasks/:taskId', () => {
    it('should return task details', async () => {
      const mockTask = {
        taskId: 'test-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: 'test-env',
        status: 'open' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.spyOn(app.services.task, 'getTask').mockResolvedValueOnce(mockTask)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/test-123',
      })

      expect(response.statusCode).toBe(200)
      const task = JSON.parse(response.body)
      expect(task.taskId).toBe('test-123')
      expect(task.agentType).toBe('claude-code')
    })

    it('should return 404 for non-existent task', async () => {
      vi.spyOn(app.services.task, 'getTask').mockResolvedValueOnce(null)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/non-existent-id',
      })

      expect(response.statusCode).toBe(404)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('TASK_NOT_FOUND')
    })
  })

  describe('GET /api/tasks', () => {
    it('should list tasks with pagination', async () => {
      const mockTasks = [
        {
          taskId: 'task-1',
          createdAt: new Date(),
          lastMessageAt: new Date(),
          agentType: 'claude-code',
          environmentId: 'test-env',
          status: 'open' as const,
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        },
        {
          taskId: 'task-2',
          createdAt: new Date(),
          lastMessageAt: new Date(),
          agentType: 'claude-code',
          environmentId: 'test-env',
          status: 'closed' as const,
          numTurns: 5,
          totalCostUsd: 0.1,
          iterations: [],
        },
      ]

      vi.spyOn(app.services.task, 'listTasks').mockResolvedValueOnce(mockTasks)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks?limit=10&offset=0',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.tasks).toHaveLength(2)
      expect(body.pagination).toBeDefined()
      expect(body.pagination.limit).toBe(10)
      expect(body.pagination.offset).toBe(0)
    })

    it('should filter tasks by status', async () => {
      const mockTasks = [
        {
          taskId: 'task-1',
          status: 'open' as const,
          createdAt: new Date(),
          lastMessageAt: new Date(),
          agentType: 'claude-code',
          environmentId: 'test-env',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        },
      ]

      vi.spyOn(app.services.task, 'listTasks').mockResolvedValueOnce(mockTasks)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks?status=running',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.tasks).toHaveLength(1)
      expect(body.tasks[0].status).toBe('running')
    })
  })

  describe('POST /api/tasks/:taskId/messages', () => {
    it('should send a message to task', async () => {
      vi.spyOn(app.services.task, 'sendMessage').mockResolvedValueOnce()

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/tasks/test-123/messages',
        payload: {
          message: 'Test message',
        },
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.status).toBe('sent')
      expect(body.timestamp).toBeDefined()
    })

    it('should validate message body', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/tasks/test-123/messages',
        payload: {
          message: '', // Empty message
        },
      })

      expect(response.statusCode).toBe(400)
    })
  })

  describe('GET /api/tasks/:taskId/events', () => {
    it('should return task events with pagination', async () => {
      const mockTask = {
        taskId: 'test-123',
        status: 'running' as const,
      } as any

      const mockEvents = [
        {
          eventId: 'evt-1',
          type: 'start',
          taskId: 'test-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
        {
          eventId: 'evt-2',
          type: 'message',
          taskId: 'test-123',
          timestamp: new Date(),
          isError: false,
          payload: { content: 'Hello' },
        },
      ] as any

      vi.spyOn(app.services.task, 'getTask').mockResolvedValueOnce(mockTask)
      vi.spyOn(app.services.event, 'getEvents').mockResolvedValueOnce(mockEvents)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/test-123/events?limit=10',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.events).toHaveLength(2)
      expect(body.items).toHaveLength(2)
    })

    it('should return 404 if task does not exist', async () => {
      vi.spyOn(app.services.task, 'getTask').mockResolvedValueOnce(null)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/non-existent/events',
      })

      expect(response.statusCode).toBe(404)
    })
  })

  describe('PATCH /api/tasks/:taskId', () => {
    it('should update task status', async () => {
      vi.spyOn(app.services.task, 'updateTask').mockResolvedValueOnce()

      const response = await app.inject({
        method: 'PATCH',
        url: '/api/v1/tasks/test-123',
        payload: {
          status: 'completed',
        },
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.status).toBe('updated')
    })

    it('should validate status value', async () => {
      const response = await app.inject({
        method: 'PATCH',
        url: '/api/v1/tasks/test-123',
        payload: {
          status: 'invalid-status',
        },
      })

      expect(response.statusCode).toBe(400)
    })
  })

  describe('DELETE /api/tasks/:taskId', () => {
    it('should delete a task', async () => {
      vi.spyOn(app.services.task, 'deleteTask').mockResolvedValueOnce()

      const response = await app.inject({
        method: 'DELETE',
        url: '/api/v1/tasks/test-123',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.status).toBe('deleted')
    })

    it('should return 404 if task not found', async () => {
      vi.spyOn(app.services.task, 'deleteTask').mockRejectedValueOnce({
        code: 'TASK_NOT_FOUND',
        statusCode: 404,
        message: 'Task not found',
      })

      const response = await app.inject({
        method: 'DELETE',
        url: '/api/v1/tasks/non-existent',
      })

      expect(response.statusCode).toBe(500) // Default error handler returns 500
    })
  })
})
