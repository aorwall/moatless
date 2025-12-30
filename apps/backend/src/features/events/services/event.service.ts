import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { Event, EventType } from '@moatless/types'
import { inject, injectable, optional } from 'inversify'
import { logger } from '../../../utils'
import { EventRepository } from '../db/event.repository'
import { EventsStorage } from '../storage/events.storage'

type EventHandler = (event: Event) => void | Promise<void>

@injectable()
export class EventService extends EventEmitter {
  private eventCache: Map<string, Event[]> = new Map()
  private readonly useDatabase: boolean

  constructor(
    @inject(EventsStorage) private storage: EventsStorage,
    @inject(EventRepository) @optional() private eventRepository?: EventRepository,
  ) {
    super()
    this.setMaxListeners(100)
    // Use database when PostgreSQL is configured and repository is available
    this.useDatabase =
      process.env['DATABASE_PROVIDER'] === 'postgres' && this.eventRepository !== undefined
    if (this.useDatabase) {
      logger.info('EventService: Using PostgreSQL for event storage')
    }
  }

  async emitEvent(event: Event): Promise<void> {
    // Ensure event has required fields
    if (!event.eventId) {
      event.eventId = randomUUID()
    }
    if (!event.timestamp) {
      event.timestamp = new Date()
    }

    if (this.useDatabase && this.eventRepository) {
      // PostgreSQL mode: save directly to database
      await this.eventRepository.saveEvent(event)

      // Update in-memory cache for real-time access
      if (!this.eventCache.has(event.taskId)) {
        this.eventCache.set(event.taskId, [])
      }
      this.eventCache.get(event.taskId)?.push(event)
    } else {
      // File storage mode: existing logic
      // Get cached events for this task or initialize
      if (!this.eventCache.has(event.taskId)) {
        const existingEvents = await this.storage.getEvents(event.taskId)
        this.eventCache.set(event.taskId, existingEvents)
      }

      // Append to cache
      const cachedEvents = this.eventCache.get(event.taskId)
      if (!cachedEvents) {
        throw new Error(`Event cache not found for task ${event.taskId}`)
      }
      cachedEvents.push(event)

      // Only save to disk if event is not a claude-code agent/task/taskIteration event
      const isClaudeCodeEvent =
        (event.type === 'agent' || event.type === 'task' || event.type === 'taskIteration') &&
        'agentType' in event &&
        event.agentType === 'claude-code'
      if (!isClaudeCodeEvent) {
        await this.storage.saveEvents(event.taskId, cachedEvents)
      }
    }

    // Emit for real-time subscribers
    super.emit(`task:${event.taskId}`, event)

    // Also emit to global channel
    super.emit('global', event)
  }

  async getEvents(
    taskId: string,
    limit?: number,
    offset = 0,
    eventType?: EventType,
  ): Promise<Event[]> {
    let events: Event[]

    // Check cache first
    const cachedEvents = this.eventCache.get(taskId)
    if (cachedEvents) {
      events = cachedEvents
    } else if (this.useDatabase && this.eventRepository) {
      // PostgreSQL mode: read from database
      events = await this.eventRepository.getEventsForTask(taskId)
      this.eventCache.set(taskId, events)
    } else {
      // File storage mode: read from file
      events = await this.storage.getEvents(taskId)
      this.eventCache.set(taskId, events)
    }

    // Filter by event type if specified
    if (eventType) {
      events = events.filter((event) => event.type === eventType)
    }

    // Apply pagination
    return events.slice(offset, limit ? offset + limit : undefined)
  }

  subscribe(taskId: string, handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch((error) => {
        logger.error({ err: error }, 'Event handler error')
      })
    }

    this.on(`task:${taskId}`, wrappedHandler)

    return () => {
      this.off(`task:${taskId}`, wrappedHandler)
    }
  }

  subscribeAll(handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch((error) => {
        logger.error({ err: error }, 'Event handler error')
      })
    }

    this.on('global', wrappedHandler)

    return () => {
      this.off('global', wrappedHandler)
    }
  }

  async getEventCount(taskId: string, eventType?: EventType): Promise<number> {
    const events = await this.getEvents(taskId, undefined, 0, eventType)
    return events.length
  }

  async deleteEvents(taskId: string): Promise<void> {
    // Clear from cache
    this.eventCache.delete(taskId)

    // Delete from appropriate storage
    if (this.useDatabase && this.eventRepository) {
      await this.eventRepository.deleteEventsForTask(taskId)
    } else {
      await this.storage.deleteEvents(taskId)
    }
  }
}
