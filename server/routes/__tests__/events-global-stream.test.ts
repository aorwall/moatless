import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Event } from '../../../shared/types/event.js'
import { EventType } from '../../../shared/types/event.js'
import type { Task } from '../../../shared/types/task.js'
import { build } from '../../test/helper.js'
import { sseManager } from '../../utils/sse-manager.js'

// Mock SSE manager
vi.mock('../../utils/sse-manager', () => ({
  sseManager: {
    connectGlobal: vi.fn(),
    disconnectGlobal: vi.fn(),
    getGlobalConnectionCount: vi.fn(),
    getAllTaskConnectionCounts: vi.fn(),
  },
}))

describe('Global Event Stream', () => {
  let app: FastifyInstance
  let mockTaskService: any
  let mockEventService: any

  const mockTask: Task = {
    taskId: 'test-task-123',
    repositoryId: 'test-env',
    agentType: 'test-agent',
    status: 'open',
    createdAt: new Date('2024-01-01'),
    lastMessageAt: new Date('2024-01-01'),
    iterations: [],
    numTurns: 0,
    totalCostUsd: 0,
  }

  const mockEvents: Event[] = [
    {
      eventId: 'event-1',
      taskId: 'test-task-123',
      type: EventType.MESSAGE,
      timestamp: new Date('2024-01-01T10:00:00Z'),
      agentType: 'test-agent',
      isError: false,
      payload: { content: 'Test message 1' },
    },
    {
      eventId: 'event-2',
      taskId: 'test-task-123',
      type: EventType.TOOL,
      subType: 'start',
      timestamp: new Date('2024-01-01T10:01:00Z'),
      agentType: 'test-agent',
      isError: false,
      payload: { tool: 'test-tool' },
    },
    {
      eventId: 'event-3',
      taskId: 'test-task-456',
      type: EventType.START,
      timestamp: new Date('2024-01-01T10:02:00Z'),
      agentType: 'test-agent',
      isError: false,
      payload: { taskId: 'test-task-456' },
    },
  ]

  beforeEach(async () => {
    // Create mock services
    mockTaskService = {
      listTasks: vi.fn().mockResolvedValue([mockTask]),
      getTask: vi.fn().mockResolvedValue(mockTask),
    }

    mockEventService = {
      getEvents: vi.fn().mockResolvedValue(mockEvents),
      subscribeAll: vi.fn(),
      emitEvent: vi.fn(),
    }

    // Build app with mocked services
    app = await build()
    // Override only the services we need for testing
    if (app.services) {
      app.services.task = mockTaskService
      app.services.event = mockEventService
    }

    // Reset all mocks
    vi.clearAllMocks()
  })

  afterEach(async () => {
    await app.close()
  })

  describe('GET /api/v1/events/stream', () => {
    it.skip('should establish SSE connection and set proper headers', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream',
        headers: {
          Accept: 'text/event-stream',
        },
      })

      expect(response.statusCode).toBe(200)
      expect(response.headers['content-type']).toBe('application/x-ndjson')
      expect(response.headers['cache-control']).toBe('no-cache')
      expect(response.headers.connection).toBe('keep-alive')
      expect(response.headers['x-accel-buffering']).toBe('no')
    })

    it.skip('should register global connection with SSE manager', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream',
      })

      expect(sseManager.connectGlobal).toHaveBeenCalledWith(
        expect.any(String), // connectionId
        expect.any(Object), // request
        expect.any(Object), // reply
        expect.objectContaining({
          types: undefined,
          activeTasksOnly: true,
        }),
      )
    })

    it.skip('should filter by event types when specified', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream?types=message,tool:start',
      })

      expect(sseManager.connectGlobal).toHaveBeenCalledWith(
        expect.any(String),
        expect.any(Object),
        expect.any(Object),
        expect.objectContaining({
          types: ['message', 'tool:start'],
          activeTasksOnly: true,
        }),
      )
    })

    it.skip('should handle activeTasksOnly parameter', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream?activeTasksOnly=false',
      })

      expect(sseManager.connectGlobal).toHaveBeenCalledWith(
        expect.any(String),
        expect.any(Object),
        expect.any(Object),
        expect.objectContaining({
          activeTasksOnly: false,
        }),
      )
    })

    it.skip('should send existing events when reconnecting with lastEventId', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      mockEventService.getEvents.mockResolvedValueOnce(mockEvents)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream?lastEventId=event-1',
      })

      expect(mockTaskService.listTasks).toHaveBeenCalled()
      expect(mockEventService.getEvents).toHaveBeenCalled()

      // Should contain events after the lastEventId
      const responseText = response.payload
      expect(responseText).toContain('event-2')
      expect(responseText).not.toContain('event-1') // Should not include the lastEventId event
    })

    it.skip('should subscribe to all events', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      let eventHandler: ((event: Event) => void) | undefined

      mockEventService.subscribeAll.mockImplementation((handler: (event: Event) => void) => {
        eventHandler = handler
        return () => {} // unsubscribe function
      })

      await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream',
      })

      expect(mockEventService.subscribeAll).toHaveBeenCalledWith(expect.any(Function))
      expect(eventHandler).toBeDefined()
    })

    it.skip('should handle CORS headers in development', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      const originalEnv = process.env.NODE_ENV
      process.env.NODE_ENV = 'development'

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream',
        headers: {
          Origin: 'http://localhost:3000',
        },
      })

      expect(response.headers['access-control-allow-origin']).toBe('http://localhost:3000')
      expect(response.headers['access-control-allow-credentials']).toBe('true')

      process.env.NODE_ENV = originalEnv
    })
  })

  describe('GET /api/v1/events', () => {
    it('should return paginated events', async () => {
      mockEventService.getEvents.mockResolvedValue(mockEvents)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events?limit=2&offset=0',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.payload)
      expect(body.events).toHaveLength(2)
      expect(body.items).toHaveLength(2)
    })

    it('should filter events by type', async () => {
      const messageEvents = mockEvents.filter((e) => e.type === EventType.MESSAGE)
      mockEventService.getEvents.mockResolvedValue(messageEvents)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events?type=message',
      })

      expect(response.statusCode).toBe(200)
      expect(mockEventService.getEvents).toHaveBeenCalledWith(
        expect.any(String),
        undefined,
        0,
        'message',
        expect.any(Object),
      )
    })

    it('should filter events by taskId', async () => {
      const taskEvents = mockEvents.filter((e) => e.taskId === 'test-task-123')
      mockEventService.getEvents.mockResolvedValue(taskEvents)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events?taskId=test-task-123',
      })

      expect(response.statusCode).toBe(200)
      expect(mockTaskService.getTask).toHaveBeenCalledWith('test-task-123')
      expect(mockEventService.getEvents).toHaveBeenCalledWith(
        'test-task-123',
        undefined,
        0,
        undefined,
        mockTask,
      )
    })

    it('should sort events by timestamp (newest first)', async () => {
      mockEventService.getEvents.mockResolvedValue(mockEvents)
      mockTaskService.listTasks.mockResolvedValue([mockTask])

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events',
      })

      const body = JSON.parse(response.payload)
      const timestamps = body.events.map((e: Event) => new Date(e.timestamp!).getTime())

      // Verify descending order
      for (let i = 1; i < timestamps.length; i++) {
        expect(timestamps[i - 1]).toBeGreaterThanOrEqual(timestamps[i])
      }
    })
  })

  describe('GET /api/v1/events/stream/status', () => {
    it('should return connection statistics', async () => {
      vi.mocked(sseManager.getGlobalConnectionCount).mockReturnValue(5)
      vi.mocked(sseManager.getAllTaskConnectionCounts).mockReturnValue({
        'task-1': 2,
        'task-2': 3,
        'task-3': 1,
      })

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream/status',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.payload)
      expect(body).toEqual({
        globalConnections: 5,
        taskConnections: {
          'task-1': 2,
          'task-2': 3,
          'task-3': 1,
        },
        totalConnections: 11, // 5 + 2 + 3 + 1
      })
    })
  })

  describe('Event filtering in stream', () => {
    it.skip('should filter events by type in real-time', async () => {
      // Skip: Complex SSE testing with app.inject
      let eventHandler: ((event: Event) => void) | undefined
      const responseChunks: string[] = []

      mockEventService.subscribeAll.mockImplementation((handler: (event: Event) => void) => {
        eventHandler = handler
        return () => {}
      })

      // Mock reply.raw.write to capture output
      vi.fn((chunk: string) => {
        responseChunks.push(chunk)
      })

      // Override app.inject to provide custom reply mock
      const originalInject = app.inject.bind(app)
      // @ts-expect-error - Overriding inject for testing
      app.inject = async (opts: any) => {
        const response = await originalInject(opts)
        // Intercept the handler registration
        if (opts.url.includes('/api/v1/events/stream?types=message')) {
          // Simulate event emission
          const messageEvent: Event = {
            eventId: 'test-event',
            taskId: 'test-task',
            type: EventType.MESSAGE,
            timestamp: new Date(),
            agentType: 'test-agent',
            isError: false,
            payload: { content: 'Test' },
          }
          const toolEvent: Event = {
            eventId: 'test-event-2',
            taskId: 'test-task',
            type: EventType.TOOL,
            subType: 'start',
            timestamp: new Date(),
            agentType: 'test-agent',
            isError: false,
            payload: { tool: 'test' },
          }

          // The handler should be registered by now
          if (eventHandler) {
            // Only message event should be written
            eventHandler(messageEvent)
            eventHandler(toolEvent)
          }
        }
        return response
      }

      await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream?types=message',
      })

      expect(eventHandler).toBeDefined()
    })
  })
})

describe('Event Service Global Channel Integration', () => {
  let app: FastifyInstance
  let eventService: any

  beforeEach(async () => {
    app = await build()
    eventService = app.services.event
  })

  afterEach(async () => {
    await app.close()
  })

  it('should emit events to both task-specific and global channels', async () => {
    const emitSpy = vi.spyOn(eventService, 'emit')

    const testEvent: Event = {
      eventId: 'test-123',
      taskId: 'task-456',
      type: EventType.MESSAGE,
      timestamp: new Date(),
      agentType: 'test-agent',
      isError: false,
      payload: { content: 'Test message' },
    }

    await eventService.emitEvent(testEvent)

    // Should emit to task-specific channel
    expect(emitSpy).toHaveBeenCalledWith(`task:${testEvent.taskId}`, testEvent)

    // Should also emit to global channel
    expect(emitSpy).toHaveBeenCalledWith('global', testEvent)
  })
})
