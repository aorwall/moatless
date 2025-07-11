import type { Event } from '@moatless/shared/types/event'
import { EventType } from '@moatless/shared/types/event'
import type { Task } from '@moatless/shared/types/task'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'
import { sseManager } from '../../utils/sse-manager.js'

// Mock SSE manager
vi.mock('../../utils/sse-manager', () => ({
  sseManager: {
    connectGlobal: vi.fn(),
    disconnectGlobal: vi.fn(),
    getGlobalConnectionCount: vi.fn(),
    getAllTaskConnectionCounts: vi.fn(),
    incrementGlobalEventCount: vi.fn(),
    getAllConnectionStats: vi.fn(),
  },
}))

describe('Global Event Stream', () => {
  let app: any
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

  afterEach(async () => {})

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
      expect(response.headers['content-type']).toBe('text/event-stream')
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
      )
    })

    it.skip('should ignore unknown parameters', async () => {
      // Skip: app.inject() waits for response to complete, which never happens with SSE
      // Test that unknown parameters are ignored
      await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream?unknownParam=value',
      })

      expect(sseManager.connectGlobal).toHaveBeenCalledWith(
        expect.any(String),
        expect.any(Object),
        expect.any(Object),
      )
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

  describe('GET /api/v1/events/stream/status', () => {
    it('should return connection statistics with event counts', async () => {
      vi.mocked(sseManager.getAllConnectionStats).mockReturnValue({
        global: { connectionCount: 5, totalEvents: 150 },
        tasks: {
          'task-1': { connectionCount: 2, totalEvents: 50 },
          'task-2': { connectionCount: 3, totalEvents: 75 },
          'task-3': { connectionCount: 1, totalEvents: 25 },
        },
        total: { connectionCount: 11, totalEvents: 300 },
      })

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream/status',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.payload)
      expect(body).toEqual({
        globalConnections: 5,
        globalEventCount: 150,
        taskConnections: {
          'task-1': 2,
          'task-2': 3,
          'task-3': 1,
        },
        taskEventCounts: {
          'task-1': 50,
          'task-2': 75,
          'task-3': 25,
        },
        totalConnections: 11,
        totalEventCount: 300,
      })
    })
  })
})

describe('Event Service Global Channel Integration', () => {
  let app: any
  let eventService: any

  beforeEach(async () => {
    app = await build()
    eventService = app.services.event
  })

  afterEach(async () => {})

  it('should emit events to both task-specific and global channels', async () => {
    // Spy on the EventEmitter's emit method via the prototype
    const emitSpy = vi.spyOn(Object.getPrototypeOf(Object.getPrototypeOf(eventService)), 'emit')

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
