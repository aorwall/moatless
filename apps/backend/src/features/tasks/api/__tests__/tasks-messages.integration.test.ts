import { OpenAPIHono } from '@hono/zod-openapi'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { AppBindings } from '../../../../app'
import { errorHandler } from '../../../../middleware/error-handler'

// Mock the container before importing routes
vi.mock('../../../../container/container.js', async () => {
  const mockMessageService = {
    getTaskMessages: vi.fn(),
  }

  return {
    appContainer: {
      get: vi.fn((serviceClass: any) => {
        // Return the appropriate mock based on service class name
        if (serviceClass.name === 'MessageService') {
          return mockMessageService
        }
        return {}
      }),
    },
  }
})

import { appContainer } from '../../../../container/container'
import { MessageService } from '../../../agents/services/message.service'
import { registerTaskRoutes } from '../tasks.routes'

// Helper to get the mocked MessageService
function getMockMessageService() {
  return (appContainer.get as any)(MessageService)
}

describe('Task Messages Endpoint - Integration Tests', () => {
  let app: OpenAPIHono<AppBindings>

  beforeEach(async () => {
    vi.clearAllMocks()

    app = new OpenAPIHono<AppBindings>()

    // Add error handler
    app.onError(errorHandler)

    // Add request ID to context
    app.use('*', async (c, next) => {
      c.set('requestId', 'test-request-id')
      await next()
    })

    // Register task routes
    await registerTaskRoutes(app)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('GET /api/v1/tasks/:taskId/messages', () => {
    it('should return messages for valid task', async () => {
      const taskId = 'test-task-1'
      const mockMessages = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Hello, how can I help you?',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'I can help you with your task.',
            },
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_input: { command: 'ls' },
              tool_use_id: 'tool-1',
              tool_result: {
                content: 'file1.txt\nfile2.txt',
                is_error: false,
                eventId: 'event-3',
              },
            },
          ],
          timestamp: new Date('2024-01-01T10:01:00Z'),
          eventId: 'event-2',
        },
      ]

      getMockMessageService().getTaskMessages.mockResolvedValue(mockMessages)

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      expect(result.messages).toHaveLength(2)
      expect(result.totalCount).toBe(2)

      // Check user message
      expect(result.messages[0]).toMatchObject({
        id: 'msg-1',
        role: 'user',
        content: 'Hello, how can I help you?',
        eventId: 'event-1',
      })

      // Check assistant message with tool call
      expect(result.messages[1]).toMatchObject({
        id: 'msg-2',
        role: 'assistant',
        eventId: 'event-2',
      })

      // Check that tool calls are properly structured
      expect(Array.isArray(result.messages[1].content)).toBe(true)
      expect(result.messages[1].content).toHaveLength(2)
      expect(result.messages[1].content[0]).toMatchObject({
        type: 'text',
        text: 'I can help you with your task.',
      })
      expect(result.messages[1].content[1]).toMatchObject({
        type: 'tool_use',
        tool_name: 'Bash',
        tool_input: { command: 'ls' },
        tool_use_id: 'tool-1',
      })
    })

    it('should return 404 when task does not exist', async () => {
      const taskId = 'non-existent-task'

      getMockMessageService().getTaskMessages.mockRejectedValue(
        new Error('Task not found: non-existent-task'),
      )

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(404)

      const result = (await response.json()) as any
      expect(result.error).toBeDefined()
      expect(result.error.code).toBe('TASK_NOT_FOUND')
      expect(result.error.message).toContain('Task non-existent-task not found')
    })

    it('should return empty array when task has no events', async () => {
      const taskId = 'task-no-events'

      getMockMessageService().getTaskMessages.mockResolvedValue([])

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      expect(result.messages).toHaveLength(0)
      expect(result.totalCount).toBe(0)
    })

    it('should handle service errors gracefully', async () => {
      const taskId = 'error-task'

      getMockMessageService().getTaskMessages.mockRejectedValue(
        new Error('Database connection failed'),
      )

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(500)

      const result = (await response.json()) as any
      expect(result.error).toBeDefined()
      expect(result.error.code).toBe('INTERNAL_SERVER_ERROR')
      expect(result.error.message).toBe('Database connection failed')
    })

    it('should filter out non-message events', async () => {
      const taskId = 'mixed-events-task'
      const mockMessages = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'User message',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: 'Assistant message',
          timestamp: new Date('2024-01-01T10:01:00Z'),
          eventId: 'event-3',
        },
      ]

      getMockMessageService().getTaskMessages.mockResolvedValue(mockMessages)

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      expect(result.messages).toHaveLength(2) // Only message events
      expect(result.totalCount).toBe(2)
      expect(result.messages[0].content).toBe('User message')
      expect(result.messages[1].content).toBe('Assistant message')
    })

    it('should set correct content-type header', async () => {
      const taskId = 'content-type-test'

      getMockMessageService().getTaskMessages.mockResolvedValue([])

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('application/json')
    })
  })
})
