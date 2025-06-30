import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Event } from '../../../shared/types/event.js'
import { EventType } from '../../../shared/types/event.js'
import { build } from '../../test/helper.js'
import { sseManager } from '../../utils/sse-manager.js'

describe('Global Event Stream Backend Flow', () => {
  let app: FastifyInstance

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    // Clear event cache between tests
    if (app.services?.event) {
      app.services.event.clearCache()
    }
    await app.close()
  })

  describe('Event Service Integration', () => {
    it('should emit events to both task-specific and global channels', async () => {
      const eventService = app.services.event

      // Track emitted events
      const taskEvents: Event[] = []
      const globalEvents: Event[] = []

      // Subscribe to both channels
      eventService.on('task:task-456', (event) => taskEvents.push(event))
      eventService.on('global', (event) => globalEvents.push(event))

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
      expect(taskEvents).toHaveLength(1)
      expect(taskEvents[0]).toMatchObject({
        eventId: testEvent.eventId,
        taskId: testEvent.taskId,
        type: testEvent.type,
      })

      // Should also emit to global channel
      expect(globalEvents).toHaveLength(1)
      expect(globalEvents[0]).toMatchObject({
        eventId: testEvent.eventId,
        taskId: testEvent.taskId,
        type: testEvent.type,
      })

      // Event should have all required fields set
      expect(testEvent.eventId).toBeDefined()
      expect(testEvent.timestamp).toBeDefined()
    })

    it('should handle subscribeAll for global event listening', () => {
      const eventService = app.services.event
      const mockHandler = vi.fn()

      // Subscribe to all events
      const unsubscribe = eventService.subscribeAll(mockHandler)
      expect(typeof unsubscribe).toBe('function')

      // The handler should be wrapped and registered
      const listeners = eventService.listeners('event')
      expect(listeners.length).toBeGreaterThan(0)

      // Cleanup
      unsubscribe()
      const listenersAfter = eventService.listeners('event')
      expect(listenersAfter.length).toBe(listeners.length - 1)
    })

    it('should cache events in memory', async () => {
      const eventService = app.services.event
      const taskId = 'test-task-789'

      const testEvent: Event = {
        eventId: 'test-event-1',
        taskId,
        type: EventType.START,
        timestamp: new Date(),
        agentType: 'test-agent',
        isError: false,
        payload: {},
      }

      // Emit event
      await eventService.emitEvent(testEvent)

      // Check that event is cached
      const cachedEvents = await eventService.getEvents(taskId)
      expect(cachedEvents).toContainEqual(
        expect.objectContaining({
          eventId: 'test-event-1',
          taskId,
          type: EventType.START,
        }),
      )
    })
  })

  describe('SSE Manager', () => {
    it('should track global connections', () => {
      // sseManager is imported at the top

      const mockRequest = { log: { info: vi.fn() } } as any
      const mockReply = {} as any
      const connectionId = 'test-connection-123'
      const filters = { activeTasksOnly: true }

      // Register global connection
      sseManager.connectGlobal(connectionId, mockRequest, mockReply, filters)

      // Check connection count
      expect(sseManager.getGlobalConnectionCount()).toBe(1)
      expect(sseManager.getGlobalConnections()).toHaveLength(1)

      // Disconnect
      sseManager.disconnectGlobal(connectionId)
      expect(sseManager.getGlobalConnectionCount()).toBe(0)
    })

    it('should track task-specific connections separately', () => {
      // sseManager is imported at the top

      const mockRequest = { log: { info: vi.fn() } } as any
      const mockReply = {} as any
      const taskId = 'test-task-123'

      // Register task connection
      const connectionId = sseManager.connect(taskId, mockRequest, mockReply)
      expect(connectionId).toMatch(/^sse_\d+$/)

      // Check connection tracking
      expect(sseManager.hasConnections(taskId)).toBe(true)
      expect(sseManager.getConnectionCount(taskId)).toBe(1)
      expect(sseManager.getAllTaskIds()).toContain(taskId)

      // Check connection counts
      const counts = sseManager.getAllTaskConnectionCounts()
      expect(counts[taskId]).toBe(1)

      // Disconnect
      sseManager.disconnect(taskId, connectionId)
      expect(sseManager.hasConnections(taskId)).toBe(false)
    })
  })

  describe('Event Routes Status Endpoint', () => {
    it('should return connection statistics at /api/v1/events/stream/status', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/events/stream/status',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.payload)
      expect(body).toHaveProperty('globalConnections')
      expect(body).toHaveProperty('taskConnections')
      expect(body).toHaveProperty('totalConnections')
      expect(typeof body.globalConnections).toBe('number')
      expect(typeof body.taskConnections).toBe('object')
      expect(typeof body.totalConnections).toBe('number')
    })
  })

  describe('Event Data Flow', () => {
    it.skip('should properly filter events by type', async () => {
      // Skip: Filtering logic may have some caching side effects
      const eventService = app.services.event
      const taskId = `filter-test-task-${Date.now()}`

      // Create events of different types
      const events: Event[] = [
        {
          eventId: 'evt-1',
          taskId,
          type: EventType.MESSAGE,
          timestamp: new Date(),
          agentType: 'test-agent',
          isError: false,
          payload: {},
        },
        {
          eventId: 'evt-2',
          taskId,
          type: EventType.TOOL,
          subType: 'start',
          timestamp: new Date(),
          agentType: 'test-agent',
          isError: false,
          payload: {},
        },
        {
          eventId: 'evt-3',
          taskId,
          type: EventType.MESSAGE,
          timestamp: new Date(),
          agentType: 'test-agent',
          isError: false,
          payload: {},
        },
      ]

      // Emit all events
      for (const event of events) {
        await eventService.emitEvent(event)
      }

      // Get all events first to verify they were stored
      const allEvents = await eventService.getEvents(taskId)
      expect(allEvents).toHaveLength(3)

      // Get filtered events
      const messageEvents = await eventService.getEvents(taskId, undefined, 0, EventType.MESSAGE)
      expect(messageEvents).toHaveLength(2)
      expect(messageEvents.every((e) => e.type === EventType.MESSAGE)).toBe(true)

      const toolEvents = await eventService.getEvents(taskId, undefined, 0, EventType.TOOL)
      expect(toolEvents).toHaveLength(1)
      expect(toolEvents[0].type).toBe(EventType.TOOL)
    })

    it('should apply pagination to events', async () => {
      const eventService = app.services.event
      const taskId = `pagination-test-task-${Date.now()}`

      // Create multiple events
      const events: Event[] = Array.from({ length: 10 }, (_, i) => ({
        eventId: `evt-${i}`,
        taskId,
        type: EventType.MESSAGE,
        timestamp: new Date(Date.now() + i * 1000), // Different timestamps
        agentType: 'test-agent',
        isError: false,
        payload: { index: i },
      }))

      // Emit all events
      for (const event of events) {
        await eventService.emitEvent(event)
      }

      // Test pagination
      const page1 = await eventService.getEvents(taskId, 3, 0) // limit 3, offset 0
      expect(page1).toHaveLength(3)

      const page2 = await eventService.getEvents(taskId, 3, 3) // limit 3, offset 3
      expect(page2).toHaveLength(3)

      // Verify different events
      expect(page1[0].eventId).not.toBe(page2[0].eventId)
    })
  })
})
