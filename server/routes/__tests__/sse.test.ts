import type { Task } from '@shared/types/task.js'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TaskService } from '../../services/task.service.js'
import { build } from '../../test/helper.js'

describe('SSE Routes', () => {
  let app: Awaited<ReturnType<typeof build>>
  let taskService: TaskService

  beforeEach(async () => {
    app = await build()
    taskService = app.services.task
  })

  afterEach(async () => {
    await app.close()
  })

  describe('GET /api/tasks/:taskId/stream', () => {
    it('should return 404 for non-existent task', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/non-existent/stream',
      })

      expect(response.statusCode).toBe(404)
    })
  })

  describe('GET /api/tasks/:taskId/stream/status', () => {
    it('should return stream status', async () => {
      const taskId = 'test-task-id-3'

      // Mock the task service to avoid creating real workspace
      vi.spyOn(taskService, 'createTask').mockResolvedValueOnce(taskId)

      // Create task through mock
      await taskService.createTask({
        agentType: 'test',
        environmentId: 'test-env',
      })

      // Mock getTask to verify task exists
      vi.spyOn(taskService, 'getTask').mockResolvedValueOnce({
        taskId,
        status: 'running',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test',
        environmentId: 'test-env',
        numTurns: 0,
        totalCostUsd: 0,
      } as Task)

      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/stream/status`,
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body).toHaveProperty('activeConnections', 0)
      expect(body).toHaveProperty('taskActive', true)
    })

    it('should return 404 for non-existent task', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/non-existent/stream/status',
      })

      expect(response.statusCode).toBe(404)
    })
  })
})
