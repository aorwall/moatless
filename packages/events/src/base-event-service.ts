import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { Event } from '@moatless/types'
import type {
  EventHandler,
  EventLogger,
  EventServiceOptions,
  EventStorage,
  EventTransport,
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
 * - Deduplication to prevent infinite loops (60s TTL)
 * - File-based event storage
 * - Optional transport for cross-process event delivery (Redis Streams)
 * - Task-specific and global event subscriptions
 */
export class BaseEventService extends EventEmitter {
  protected readonly logger: EventLogger

  // Deduplication: Track recently emitted eventIds
  protected recentEventIds = new Map<string, number>()
  protected readonly eventTtlMs: number
  protected readonly cleanupInterval: number
  protected emitCounter = 0

  // Storage implementations
  protected storage?: EventStorage
  protected transport?: EventTransport

  constructor(options: EventServiceOptions = {}) {
    super()
    this.setMaxListeners(options.maxListeners ?? 100)
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
   * Set the transport implementation for cross-process event delivery
   */
  setTransport(transport: EventTransport): void {
    this.transport = transport
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

    // Save to storage (file-based only)
    await this.saveEvent(event)

    // Publish to transport if configured (worker mode)
    if (this.transport) {
      await this.transport.publishEvent(event)
    }

    // Emit for real-time subscribers (main app in-process)
    super.emit(`task:${event.taskId}`, event)

    // Also emit to global channel
    super.emit('global', event)
  }

  /**
   * Save event to storage (can be overridden)
   */
  protected async saveEvent(event: Event): Promise<void> {
    // Save to file storage if configured
    if (this.storage) {
      const existingEvents = await this.storage.getEvents(event.taskId)
      await this.storage.saveEvents(event.taskId, [...existingEvents, event])
    }
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
