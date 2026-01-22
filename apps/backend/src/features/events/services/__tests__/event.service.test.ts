import { randomUUID } from 'node:crypto'
import type { Event } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EventService } from '../event.service'

describe('EventService', () => {
  let container: Container
  let eventService: EventService

  beforeEach(() => {
    container = new Container()
    container.bind(EventService).toSelf()
    eventService = container.get(EventService)
  })

  afterEach(() => {
    container.unbindAll()
  })

  describe('emitEvent', () => {
    it('should assign eventId and timestamp if missing', async () => {
      const taskId = 'task-123'

      const event: any = {
        type: 'task' as const,
        subType: 'created' as const,
        taskId,
      } as Event

      await eventService.emitEvent(event)

      expect(event.eventId).toBeDefined()
      expect(event.timestamp).toBeDefined()
    })

    it('should emit event to task-specific and global channels', async () => {
      const taskId = 'task-123'

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

    it('should deduplicate events with the same eventId', async () => {
      const taskId = 'task-123'
      const eventId = randomUUID()

      const handler = vi.fn()
      eventService.subscribeAll(handler)

      const event: any = {
        eventId,
        type: 'task',
        subType: 'created',
        taskId,
        timestamp: new Date(),
      }

      // Emit the same event twice
      await eventService.emitEvent(event)
      await eventService.emitEvent(event)

      // Wait for async event handlers
      await new Promise((resolve) => setTimeout(resolve, 10))

      // Should only be called once due to deduplication
      expect(handler).toHaveBeenCalledTimes(1)
    })
  })

  describe('subscribe', () => {
    it('should allow subscription to task-specific events', async () => {
      const taskId = 'task-123'
      const handler = vi.fn()

      const unsubscribe = eventService.subscribe(taskId, handler)

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
})
