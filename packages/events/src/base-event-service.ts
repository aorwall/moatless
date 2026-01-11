import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { Event, EventType } from '@moatless/types'
import type {
  EventHandler,
  EventLogger,
  EventQueryOptions,
  EventRepository,
  EventServiceOptions,
  EventStorage,
} from './types'

/**
 * Default logger that does nothing
 */
const noopLogger: EventLogger = {
  debug: () => undefined,
  info: () => undefined,
  warn: () => undefined,
  error: () => undefined,
}

/**
 * BaseEventService - Core event emitter with caching and deduplication
 *
 * This class provides the core functionality for event emission, caching,
 * and subscription management. It can be extended or used directly by
 * both the main server and sandbox apps.
 *
 * Features:
 * - In-memory event caching per task
 * - Deduplication to prevent infinite loops
 * - Support for both file and database storage
 * - Task-specific and global event subscriptions
 */
export class BaseEventService extends EventEmitter {
  protected eventCache: Map<string, Event[]> = new Map()
  protected readonly useDatabase: boolean
  protected readonly markDeliveredOnSave: boolean
  protected readonly logger: EventLogger

  // Deduplication: Track recently emitted eventIds
  protected recentEventIds = new Map<string, number>()
  protected readonly eventTtlMs: number
  protected readonly cleanupInterval: number
  protected emitCounter = 0

  // Storage implementations
  protected storage?: EventStorage
  protected repository?: EventRepository

  constructor(options: EventServiceOptions = {}) {
    super()
    this.setMaxListeners(options.maxListeners ?? 100)
    this.useDatabase = options.useDatabase ?? false
    this.markDeliveredOnSave = options.markDeliveredOnSave ?? true
    this.eventTtlMs = options.deduplicationTtlMs ?? 60000
    this.cleanupInterval = options.cleanupInterval ?? 100
    this.logger = options.logger ?? noopLogger
  }

  /**
   * Set the file storage implementation
   */
  setStorage(storage: EventStorage): void {
    this.storage = storage
  }

  /**
   * Set the database repository implementation
   */
  setRepository(repository: EventRepository): void {
    this.repository = repository
  }

  /**
   * Emit an event
   *
   * @param event - The event to emit
   */
  async emitEvent(event: Event): Promise<void> {
    // Ensure event has required fields
    if (!event.eventId) {
      event.eventId = randomUUID()
    }
    if (!event.timestamp) {
      event.timestamp = new Date()
    }

    // Deduplication: Skip if we've already processed this exact event
    if (this.recentEventIds.has(event.eventId)) {
      this.logger.debug(
        { eventId: event.eventId, taskId: event.taskId, type: event.type },
        'Duplicate event skipped (already processed)',
      )
      return
    }

    // Track this event and periodically clean up expired entries
    this.recentEventIds.set(event.eventId, Date.now())
    this.emitCounter++
    if (this.emitCounter >= this.cleanupInterval) {
      this.cleanupExpiredEvents()
      this.emitCounter = 0
    }

    // Save to storage
    await this.saveEvent(event)

    // Emit for real-time subscribers
    super.emit(`task:${event.taskId}`, event)

    // Also emit to global channel
    super.emit('global', event)
  }

  /**
   * Save event to storage (can be overridden)
   */
  protected async saveEvent(event: Event): Promise<void> {
    if (this.useDatabase && this.repository) {
      // PostgreSQL mode: save directly to database
      if (this.markDeliveredOnSave) {
        await this.repository.saveEvent(event)
      } else {
        await this.repository.savePendingEvent(event)
      }

      // Update in-memory cache
      this.addToCache(event)
    } else if (this.storage) {
      // File storage mode
      const cachedEvents = await this.ensureCacheForTask(event.taskId)
      cachedEvents.push(event)

      // Save to disk (can be optimized by subclasses)
      await this.storage.saveEvents(event.taskId, cachedEvents)
    } else {
      // No storage - just cache
      this.addToCache(event)
    }
  }

  /**
   * Get events for a task
   */
  async getEvents(taskId: string, options: EventQueryOptions = {}): Promise<Event[]> {
    const { limit, offset = 0, eventType } = options
    let events: Event[]

    // Check cache first
    const cachedEvents = this.eventCache.get(taskId)
    if (cachedEvents) {
      events = cachedEvents
    } else if (this.useDatabase && this.repository) {
      // PostgreSQL mode
      events = await this.repository.getEventsForTask(taskId)
      this.eventCache.set(taskId, events)
    } else if (this.storage) {
      // File storage mode
      events = await this.storage.getEvents(taskId)
      this.eventCache.set(taskId, events)
    } else {
      events = []
    }

    // Filter by event type if specified
    if (eventType) {
      events = events.filter((event) => event.type === eventType)
    }

    // Apply pagination
    return events.slice(offset, limit ? offset + limit : undefined)
  }

  /**
   * Subscribe to events for a specific task
   */
  subscribe(taskId: string, handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch((error) => {
        this.logger.error({ err: error }, 'Event handler error')
      })
    }

    this.on(`task:${taskId}`, wrappedHandler)

    return () => {
      this.off(`task:${taskId}`, wrappedHandler)
    }
  }

  /**
   * Subscribe to all events (global)
   */
  subscribeAll(handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch((error) => {
        this.logger.error({ err: error }, 'Event handler error')
      })
    }

    this.on('global', wrappedHandler)

    return () => {
      this.off('global', wrappedHandler)
    }
  }

  /**
   * Get event count for a task
   */
  async getEventCount(taskId: string, eventType?: EventType): Promise<number> {
    const events = await this.getEvents(taskId, { eventType })
    return events.length
  }

  /**
   * Delete events for a task
   */
  async deleteEvents(taskId: string): Promise<void> {
    // Clear from cache
    this.eventCache.delete(taskId)

    // Delete from appropriate storage
    if (this.useDatabase && this.repository) {
      await this.repository.deleteEventsForTask(taskId)
    } else if (this.storage) {
      await this.storage.deleteEvents(taskId)
    }
  }

  /**
   * Clear in-memory cache for a task
   */
  clearCache(taskId: string): void {
    this.eventCache.delete(taskId)
  }

  /**
   * Clear all in-memory caches
   */
  clearAllCaches(): void {
    this.eventCache.clear()
    this.recentEventIds.clear()
  }

  /**
   * Add event to in-memory cache
   */
  protected addToCache(event: Event): void {
    if (!this.eventCache.has(event.taskId)) {
      this.eventCache.set(event.taskId, [])
    }
    this.eventCache.get(event.taskId)?.push(event)
  }

  /**
   * Ensure cache exists for a task, loading from storage if needed
   */
  protected async ensureCacheForTask(taskId: string): Promise<Event[]> {
    if (!this.eventCache.has(taskId)) {
      if (this.storage) {
        const existingEvents = await this.storage.getEvents(taskId)
        this.eventCache.set(taskId, existingEvents)
      } else {
        this.eventCache.set(taskId, [])
      }
    }
    return this.eventCache.get(taskId)!
  }

  /**
   * Remove expired event IDs from deduplication cache
   */
  protected cleanupExpiredEvents(): void {
    const now = Date.now()
    let expiredCount = 0

    for (const [eventId, timestamp] of this.recentEventIds) {
      if (now - timestamp > this.eventTtlMs) {
        this.recentEventIds.delete(eventId)
        expiredCount++
      }
    }

    if (expiredCount > 0) {
      this.logger.debug(
        { expiredCount, remaining: this.recentEventIds.size },
        'Cleaned up expired event IDs from deduplication cache',
      )
    }
  }
}
