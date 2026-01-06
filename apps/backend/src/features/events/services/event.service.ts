import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { Event, EventType } from '@moatless/types'
import { inject, injectable, optional } from 'inversify'
import { logger } from '../../../utils'
import { eventEmittedTotal } from '../../metrics'
import { EventRepository } from '../db/event.repository'
import { EventsStorage } from '../storage/events.storage'

type EventHandler = (event: Event) => void | Promise<void>

@injectable()
export class EventService extends EventEmitter {
  private eventCache: Map<string, Event[]> = new Map()
  private readonly useDatabase: boolean

  // Deduplication: Track recently emitted eventIds to prevent loops
  private recentEventIds = new Map<string, number>() // eventId → timestamp
  private static readonly EVENT_TTL_MS = 60000 // 1 minute
  private static readonly CLEANUP_INTERVAL = 100 // Clean up every 100 events

  private emitCounter = 0

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

    // Deduplication: Skip if we've already processed this exact event
    // This prevents infinite loops where the same event gets re-emitted
    if (this.recentEventIds.has(event.eventId)) {
      logger.debug(
        { eventId: event.eventId, taskId: event.taskId, type: event.type },
        'Duplicate event skipped (already processed)',
      )
      return
    }

    // Track this event and periodically clean up expired entries
    this.recentEventIds.set(event.eventId, Date.now())
    this.emitCounter++
    if (this.emitCounter >= EventService.CLEANUP_INTERVAL) {
      this.cleanupExpiredEvents()
      this.emitCounter = 0
    }

    // Track event emission
    const storageMode = this.useDatabase ? 'postgres' : 'file'
    eventEmittedTotal.labels({ type: event.type, storage_mode: storageMode }).inc()

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

  /**
   * Remove expired event IDs from deduplication cache.
   * Called periodically to prevent unbounded memory growth.
   */
  private cleanupExpiredEvents(): void {
    const now = Date.now()
    let expiredCount = 0

    for (const [eventId, timestamp] of this.recentEventIds) {
      if (now - timestamp > EventService.EVENT_TTL_MS) {
        this.recentEventIds.delete(eventId)
        expiredCount++
      }
    }

    if (expiredCount > 0) {
      logger.debug(
        { expiredCount, remaining: this.recentEventIds.size },
        'Cleaned up expired event IDs from deduplication cache',
      )
    }
  }
}
