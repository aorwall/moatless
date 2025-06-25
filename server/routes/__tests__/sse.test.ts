import { EventType } from '@shared/types/event.js'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { EventService } from '../../services/event.service.js'
import type { TaskService } from '../../services/task.service.js'
import { build } from '../../test/helper.js'

// import type { WorkspaceService } from '../../services/workspace.service.js'

describe('SSE Routes', () => {
  let app: any
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
    it('should stream events as NDJSON', async () => {
      const taskId = await taskService.createTask({
        agentType: 'test',
        environmentId: 'test-env',
      })

      const response = await app.inject({
        method: 'GET',
        url: `/api/tasks/${taskId}/stream`,
        headers: {
          Accept: 'application/x-ndjson',
        },
      })

      expect(response.statusCode).toBe(200)
      expect(response.headers['content-type']).toBe('application/x-ndjson')
      expect(response.headers['cache-control']).toBe('no-cache')
      expect(response.headers['connection']).toBe('keep-alive')
    })

    it('should return 404 for non-existent task', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/non-existent/stream',
      })

      expect(response.statusCode).toBe(404)
    })

    it('should support lastEventId for reconnection', async () => {
      const taskId = await taskService.createTask({
        agentType: 'test',
        environmentId: 'test-env',
      })

      // Add some events
      await eventService.emitEvent({
        type: EventType.MESSAGE,
        taskId,
        eventId: 'event-2',
        timestamp: new Date(),
        isError: false,
        payload: { content: 'Event 1' },
      })

      const response = await app.inject({
        method: 'GET',
        url: `/api/tasks/${taskId}/stream?lastEventId=event-1`,
      })

      expect(response.statusCode).toBe(200)
    })
  })

  describe('GET /api/tasks/:taskId/stream/status', () => {
    it('should return stream status', async () => {
      const taskId = await taskService.createTask({
        agentType: 'test',
        environmentId: 'test-env',
      })

      const response = await app.inject({
        method: 'GET',
        url: `/api/tasks/${taskId}/stream/status`,
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body).toHaveProperty('active_connections', 0)
      expect(body).toHaveProperty('task_active', true)
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
