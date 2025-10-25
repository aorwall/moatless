import { randomUUID } from 'node:crypto'
import type { Event } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EventsStorage } from '../../storage/events.storage.js'
import { EventService } from '../event.service.js'

describe('EventService', () => {
  let container: Container
  let eventService: EventService
  let mockEventsStorage: EventsStorage

  beforeEach(() => {
    container = new Container()

    mockEventsStorage = {
      saveEvents: vi.fn(),
      getEvents: vi.fn(),
    } as unknown as EventsStorage

    container.bind(EventsStorage).toConstantValue(mockEventsStorage)
    container.bind(EventService).toSelf()

    eventService = container.get(EventService)
  })

  afterEach(() => {
    container.unbindAll()
  })

  describe('emitEvent', () => {
    it('should assign eventId and timestamp if missing', async () => {
      const taskId = 'task-123'
      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue([])

      const event: any = {
        type: 'task' as const,
        subType: 'created' as const,
        taskId,
      } as Event

      await eventService.emitEvent(event)

      expect(event.eventId).toBeDefined()
      expect(event.timestamp).toBeDefined()
      expect(mockEventsStorage.saveEvents).toHaveBeenCalled()
    })

    it('should cache events for a task', async () => {
      const taskId = 'task-123'
      const existingEvents: any[] = []
      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue(existingEvents)

      const event: any = {
        eventId: randomUUID(),
        type: 'task',
        subType: 'created',
        taskId,
        timestamp: new Date(),
      }

      await eventService.emitEvent(event)

      // Should have called getEvents to initialize cache
      expect(mockEventsStorage.getEvents).toHaveBeenCalledWith(taskId)
      // Should have saved updated events
      expect(mockEventsStorage.saveEvents).toHaveBeenCalledWith(taskId, [event])
    })

    it('should not save Claude Code events to disk', async () => {
      const taskId = 'task-123'
      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue([])

      const claudeEvent: any = {
        eventId: randomUUID(),
        type: 'agent',
        subType: 'started',
        taskId,
        timestamp: new Date(),
        agentType: 'claude-code',
      }

      await eventService.emitEvent(claudeEvent)

      // Should initialize cache but NOT save to disk
      expect(mockEventsStorage.getEvents).toHaveBeenCalledWith(taskId)
      expect(mockEventsStorage.saveEvents).not.toHaveBeenCalled()
    })

    it('should emit event to task-specific and global channels', async () => {
      const taskId = 'task-123'
      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue([])

      const taskHandler = vi.fn()
      const globalHandler = vi.fn()

      eventService.subscribe(taskId, taskHandler)
      eventService.subscribeAll(globalHandler)

      const event: any = {
        eventId: randomUUID(),
        type: 'task',
        subType: 'created',
        taskId,
        timestamp: new Date(),
      }

      await eventService.emitEvent(event)

      // Wait for async event handlers
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(taskHandler).toHaveBeenCalledWith(event)
      expect(globalHandler).toHaveBeenCalledWith(event)
    })
  })

  describe('getEvents', () => {
    it('should return events from cache if available', async () => {
      const taskId = 'task-123'
      const cachedEvents: any[] = [
        {
          eventId: randomUUID(),
          type: 'task',
          subType: 'created',
          taskId,
          timestamp: new Date(),
        },
      ]

      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue(cachedEvents)

      // First call should load from storage and cache
      await eventService.emitEvent(cachedEvents[0])

      // Second call should use cache
      const result = await eventService.getEvents(taskId)

      // Event was added to the existing cache, so we have 2 events now
      expect(result).toHaveLength(2)
      expect(mockEventsStorage.getEvents).toHaveBeenCalledTimes(1) // Only once for initial load
    })

    it('should filter events by type when specified', async () => {
      const taskId = 'task-123'
      const events: any[] = [
        {
          eventId: randomUUID(),
          type: 'task',
          subType: 'created',
          taskId,
          timestamp: new Date(),
        },
        {
          eventId: randomUUID(),
          type: 'workspace',
          subType: 'ready',
          taskId,
          timestamp: new Date(),
        },
      ]

      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue(events)

      const result = await eventService.getEvents(taskId, undefined, 0, 'task')

      expect(result).toHaveLength(1)
      expect(result[0]?.type).toBe('task')
    })

    it('should apply pagination correctly', async () => {
      const taskId = 'task-123'
      const events: any[] = Array.from({ length: 10 }, () => ({
        eventId: randomUUID(),
        type: 'task' as const,
        subType: 'created' as const,
        taskId,
        timestamp: new Date(),
      }))

      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue(events)

      const result = await eventService.getEvents(taskId, 3, 2)

      expect(result).toHaveLength(3)
      expect(result).toEqual(events.slice(2, 5))
    })
  })

  describe('subscribe', () => {
    it('should allow subscription to task-specific events', async () => {
      const taskId = 'task-123'
      const handler = vi.fn()

      const unsubscribe = eventService.subscribe(taskId, handler)

      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue([])

      const event: any = {
        eventId: randomUUID(),
        type: 'task',
        subType: 'created',
        taskId,
        timestamp: new Date(),
      }

      await eventService.emitEvent(event)

      // Wait for async handler
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(handler).toHaveBeenCalledWith(event)

      // Unsubscribe and verify no more calls
      unsubscribe()
      handler.mockClear()

      await eventService.emitEvent({ ...event, eventId: randomUUID() })
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(handler).not.toHaveBeenCalled()
    })
  })

  describe('subscribeAll', () => {
    it('should receive events from all tasks', async () => {
      const handler = vi.fn()
      eventService.subscribeAll(handler)

      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue([])

      const event1: any = {
        eventId: randomUUID(),
        type: 'task',
        subType: 'created',
        taskId: 'task-1',
        timestamp: new Date(),
      }

      const event2: any = {
        eventId: randomUUID(),
        type: 'task',
        subType: 'created',
        taskId: 'task-2',
        timestamp: new Date(),
      }

      await eventService.emitEvent(event1)
      await eventService.emitEvent(event2)

      // Wait for async handlers
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(handler).toHaveBeenCalledTimes(2)
      expect(handler).toHaveBeenCalledWith(event1)
      expect(handler).toHaveBeenCalledWith(event2)
    })
  })

  describe('getEventCount', () => {
    it('should return total event count', async () => {
      const taskId = 'task-123'
      const events: any[] = Array.from({ length: 5 }, () => ({
        eventId: randomUUID(),
        type: 'task' as const,
        subType: 'created' as const,
        taskId,
        timestamp: new Date(),
      }))

      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue(events)

      const count = await eventService.getEventCount(taskId)

      expect(count).toBe(5)
    })

    it('should return filtered event count when type specified', async () => {
      const taskId = 'task-123'
      const events: any[] = [
        ...Array.from({ length: 3 }, () => ({
          eventId: randomUUID(),
          type: 'task' as const,
          subType: 'created' as const,
          taskId,
          timestamp: new Date(),
        })),
        ...Array.from({ length: 2 }, () => ({
          eventId: randomUUID(),
          type: 'workspace' as const,
          subType: 'ready' as const,
          taskId,
          timestamp: new Date(),
        })),
      ]

      ;(mockEventsStorage.getEvents as ReturnType<typeof vi.fn>).mockResolvedValue(events)

      const count = await eventService.getEventCount(taskId, 'task')

      expect(count).toBe(3)
    })
  })
})
