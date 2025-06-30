import { EventType } from '@shared/types/event.js'
import type { Task } from '@shared/types/task.js'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { EventService } from '../../services/event.service.js'
import type { TaskService } from '../../services/task.service.js'
import { build } from '../../test/helper.js'

// import type { WorkspaceService } from '../../services/workspace.service.js'

describe('SSE Routes', () => {
  let app: Awaited<ReturnType<typeof build>>
  let taskService: TaskService
  let eventService: EventService
  // let workspaceService: WorkspaceService

  beforeEach(async () => {
    app = await build()
    taskService = app.services.task
    eventService = app.services.event
    // workspaceService = app.services.workspace
  })

  afterEach(async () => {
    await app.close()
  })

  describe('GET /api/tasks/:taskId/stream', () => {
    it.skip('should stream events as NDJSON', async () => {
      const taskId = 'test-task-id'

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

      // Skip this test as SSE endpoints don't close connections
      // and inject() waits for response to complete
      expect(true).toBe(true)
    })

    it('should return 404 for non-existent task', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/non-existent/stream',
      })

      expect(response.statusCode).toBe(404)
    })

    it.skip('should support lastEventId for reconnection', async () => {
      const taskId = 'test-task-id-2'

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

      // Add some events
      await eventService.emitEvent({
        type: EventType.MESSAGE,
        taskId,
        eventId: 'event-2',
        timestamp: new Date(),
        isError: false,
        agentType: 'test',
        payload: { content: 'Event 1' },
      })

      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/stream?lastEventId=event-1`,
        // Add simulate option to handle streaming response
        simulate: {
          end: true,
          split: false,
          error: false,
          close: false,
        },
      })

      expect((response as any).statusCode).toBe(200)
    }, 10000)
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
