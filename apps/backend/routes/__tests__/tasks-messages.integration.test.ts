import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { Hono } from 'hono'
import type { Event, EventType } from '@moatless/types'
import { registerTaskRoutes } from '../tasks.js'
import type { AppBindings } from '../../app.js'

// Mock the service dependencies  
const mockEventService = {
  getTaskEvents: vi.fn()
}

const mockTaskService = {
  getTask: vi.fn()
}

const mockServices = {
  event: mockEventService,
  task: mockTaskService,
  workspace: {},
  repository: {},
  previewEnvironment: {},
  setting: {},
  claude: {}
}

describe('Task Messages Endpoint - Integration Tests', () => {
  let app: Hono<AppBindings>

  beforeEach(async () => {
    vi.clearAllMocks()
    
    app = new Hono<AppBindings>()
    
    // Add services to context
    app.use('*', async (c, next) => {
      c.set('services', mockServices as any)
      await next()
    })
    
    // Register task routes
    await registerTaskRoutes(app, '/api/v1/tasks')
  })

  afterEach(() => {
    vi.resetAllMocks()
  })

  describe('GET /api/v1/tasks/:taskId/messages', () => {
    it('should return messages for valid task', async () => {
      const taskId = 'test-task-1'
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId,
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'text',
                  text: 'Hello, how can I help you?'
                }
              ]
            }
          }
        },
        {
          eventId: 'event-2',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId,
          timestamp: new Date('2024-01-01T10:01:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'I can help you with your task.'
                },
                {
                  type: 'tool_use',
                  id: 'tool-1',
                  name: 'Bash',
                  input: { command: 'ls' }
                }
              ]
            }
          }
        },
        {
          eventId: 'event-3',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId,
          timestamp: new Date('2024-01-01T10:01:01Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'tool_result',
                  tool_use_id: 'tool-1',
                  content: 'file1.txt\nfile2.txt'
                }
              ]
            }
          }
        }
      ]

      mockTaskService.getTask.mockResolvedValue({
        taskId,
        title: 'Test Task',
        status: 'active'
      })
      mockEventService.getTaskEvents.mockResolvedValue(mockEvents)

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)
      
      const result = await response.json()
      expect(result.success).toBe(true)
      expect(result.data).toHaveLength(2)
      
      // Check user message
      expect(result.data[0]).toMatchObject({
        id: 'event-1',
        role: 'user',
        content: 'Hello, how can I help you?',
        eventId: 'event-1'
      })
      
      // Check assistant message with tool call
      expect(result.data[1]).toMatchObject({
        id: 'event-2',
        role: 'assistant',
        eventId: 'event-2'
      })
      
      // Check that tool calls are properly structured
      expect(Array.isArray(result.data[1].content)).toBe(true)
      expect(result.data[1].content).toHaveLength(2)
      expect(result.data[1].content[0]).toMatchObject({
        type: 'text',
        text: 'I can help you with your task.'
      })
      expect(result.data[1].content[1]).toMatchObject({
        type: 'tool_use',
        tool_name: 'Bash',
        tool_input: { command: 'ls' },
        tool_use_id: 'tool-1'
      })
    })

    it('should return 404 when task does not exist', async () => {
      const taskId = 'non-existent-task'
      
      mockTaskService.getTask.mockResolvedValue(null)

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(404)
      
      const result = await response.json()
      expect(result.success).toBe(false)
      expect(result.error).toContain('Task not found')
    })

    it('should return empty array when task has no events', async () => {
      const taskId = 'task-no-events'
      
      mockTaskService.getTask.mockResolvedValue({
        taskId,
        title: 'Task with no events',
        status: 'active'
      })
      mockEventService.getTaskEvents.mockResolvedValue([])

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)
      
      const result = await response.json()
      expect(result.success).toBe(true)
      expect(result.data).toHaveLength(0)
    })

    it('should handle service errors gracefully', async () => {
      const taskId = 'error-task'
      
      mockTaskService.getTask.mockRejectedValue(new Error('Database connection failed'))

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(500)
      
      const result = await response.json()
      expect(result.success).toBe(false)
      expect(result.error).toContain('Internal server error')
    })

    it('should filter out non-message events', async () => {
      const taskId = 'mixed-events-task'
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId,
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'text',
                  text: 'User message'
                }
              ]
            }
          }
        },
        {
          eventId: 'event-2',
          agentType: 'test-agent',
          type: 'task_status' as EventType,
          taskId,
          timestamp: new Date('2024-01-01T10:00:30Z'),
          isError: false,
          payload: {
            status: 'running'
          }
        },
        {
          eventId: 'event-3',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId,
          timestamp: new Date('2024-01-01T10:01:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'Assistant message'
                }
              ]
            }
          }
        }
      ]

      mockTaskService.getTask.mockResolvedValue({
        taskId,
        title: 'Mixed Events Task',
        status: 'active'
      })
      mockEventService.getTaskEvents.mockResolvedValue(mockEvents)

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)
      
      const result = await response.json()
      expect(result.success).toBe(true)
      expect(result.data).toHaveLength(2) // Only message events
      expect(result.data[0].content).toBe('User message')
      expect(result.data[1].content).toBe('Assistant message')
    })

    it('should set correct content-type header', async () => {
      const taskId = 'content-type-test'
      
      mockTaskService.getTask.mockResolvedValue({
        taskId,
        title: 'Content Type Test',
        status: 'active'
      })
      mockEventService.getTaskEvents.mockResolvedValue([])

      const response = await app.request(`/api/v1/tasks/${taskId}/messages`)

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('application/json; charset=UTF-8')
    })
  })
})