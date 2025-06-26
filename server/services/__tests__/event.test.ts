import { beforeEach, describe, expect, it, vi } from 'vitest'
import { EventType } from '../../../shared/types/event.js'
import type { Event } from '../../../shared/types/index.js'
import { EventService } from '../event.service.js'
import type { StorageBackend } from '../storage.js'

describe('EventService', () => {
  let eventService: EventService
  let mockStorage: StorageBackend

  beforeEach(() => {
    // Create mock storage
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      list: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      saveTask: vi.fn(),
      getTask: vi.fn(),
      saveEnvironment: vi.fn(),
      getEnvironment: vi.fn(),
      saveWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      saveEvents: vi.fn(),
      getEvents: vi.fn().mockResolvedValue([]),
    }

    eventService = new EventService(mockStorage)
  })

  describe('emitEvent', () => {
    it('should emit an event with auto-generated ID and timestamp', async () => {
      const event: Event = {
        eventId: '',
        agentType: 'claude-code',
        type: EventType.START,
        taskId: 'task-123',
        timestamp: null as any,
        isError: false,
        payload: { message: 'Starting task' },
      }

      await eventService.emitEvent(event)

      expect(mockStorage.saveEvents).toHaveBeenCalledWith(
        'task-123',
        expect.arrayContaining([
          expect.objectContaining({
            eventId: expect.any(String),
            timestamp: expect.any(Date),
            type: EventType.START,
            taskId: 'task-123',
          }),
        ]),
      )
    })

    it('should append event to existing events', async () => {
      const existingEvents: Event[] = [
        {
          eventId: 'evt-1',
          agentType: 'claude-code',
          type: EventType.START,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
      ]

      vi.mocked(mockStorage.getEvents).mockResolvedValueOnce(existingEvents)

      const newEvent: Event = {
        eventId: 'evt-2',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: 'task-123',
        timestamp: new Date(),
        isError: false,
        payload: { content: 'Hello' },
      }

      await eventService.emitEvent(newEvent)

      expect(mockStorage.saveEvents).toHaveBeenCalledWith(
        'task-123',
        expect.arrayContaining([
          expect.objectContaining({ eventId: 'evt-1' }),
          expect.objectContaining({ eventId: 'evt-2' }),
        ]),
      )
    })
  })

  describe('getEvents', () => {
    it('should retrieve events with pagination', async () => {
      const events: Event[] = Array.from({ length: 10 }, (_, i) => ({
        eventId: `evt-${i}`,
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: 'task-123',
        timestamp: new Date(),
        isError: false,
        payload: { index: i },
      }))

      vi.mocked(mockStorage.getEvents).mockResolvedValueOnce(events)

      const result = await eventService.getEvents('task-123', 5, 2)

      expect(result).toHaveLength(5)
      expect(result[0].eventId).toBe('evt-2')
      expect(result[4].eventId).toBe('evt-6')
    })

    it('should filter events by type', async () => {
      const events: Event[] = [
        {
          eventId: 'evt-1',
          agentType: 'claude-code',
          type: EventType.START,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
        {
          eventId: 'evt-2',
          agentType: 'claude-code',
          type: EventType.MESSAGE,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
        {
          eventId: 'evt-3',
          agentType: 'claude-code',
          type: EventType.MESSAGE,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
      ]

      vi.mocked(mockStorage.getEvents).mockResolvedValueOnce(events)

      const result = await eventService.getEvents('task-123', undefined, 0, EventType.MESSAGE)

      expect(result).toHaveLength(2)
      expect(result.every((e) => e.type === EventType.MESSAGE)).toBe(true)
    })
  })

  describe('subscribe/unsubscribe', () => {
    it('should subscribe and unsubscribe from task events', async () => {
      const handler = vi.fn()
      const unsubscribe = eventService.subscribe('task-123', handler)

      const event: Event = {
        eventId: 'evt-1',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: 'task-123',
        timestamp: new Date(),
        isError: false,
        payload: {},
      }

      await eventService.emitEvent(event)
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(handler).toHaveBeenCalledTimes(1)

      // Unsubscribe
      unsubscribe()

      await eventService.emitEvent(event)
      await new Promise((resolve) => setTimeout(resolve, 10))

      // Should still be called only once
      expect(handler).toHaveBeenCalledTimes(1)
    })

    it('should handle errors in event handlers gracefully', async () => {
      const errorHandler = vi.fn().mockRejectedValue(new Error('Handler error'))
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

      eventService.subscribe('task-123', errorHandler)

      const event: Event = {
        eventId: 'evt-1',
        agentType: 'claude-code',
        type: EventType.ERROR,
        taskId: 'task-123',
        timestamp: new Date(),
        isError: true,
        payload: { error: 'Test error' },
      }

      // Should not throw - the event emission itself should succeed
      await eventService.emitEvent(event)

      // Give time for the async error handler to run
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(errorHandler).toHaveBeenCalled()
      expect(consoleErrorSpy).toHaveBeenCalledWith(expect.any(Error))

      consoleErrorSpy.mockRestore()
    })
  })

  describe('getEventCount', () => {
    it('should count all events for a task', async () => {
      const events: Event[] = Array.from({ length: 15 }, (_, i) => ({
        eventId: `evt-${i}`,
        agentType: 'claude-code',
        type: i % 2 === 0 ? EventType.MESSAGE : EventType.TOOL,
        taskId: 'task-123',
        timestamp: new Date(),
        isError: false,
        payload: {},
      }))

      vi.mocked(mockStorage.getEvents).mockResolvedValueOnce(events)

      const count = await eventService.getEventCount('task-123')
      expect(count).toBe(15)
    })

    it('should count events by type', async () => {
      const events: Event[] = [
        {
          eventId: 'evt-1',
          agentType: 'claude-code',
          type: EventType.MESSAGE,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
        {
          eventId: 'evt-2',
          agentType: 'claude-code',
          type: EventType.TOOL,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
        {
          eventId: 'evt-3',
          agentType: 'claude-code',
          type: EventType.MESSAGE,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          payload: {},
        },
      ]

      vi.mocked(mockStorage.getEvents).mockResolvedValueOnce(events)

      const count = await eventService.getEventCount('task-123', EventType.MESSAGE)
      expect(count).toBe(2)
    })
  })
})
