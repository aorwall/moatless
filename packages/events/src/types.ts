import type { Event, EventType } from '@moatless/types'

/**
 * Event handler function type
 */
export type EventHandler = (event: Event) => void | Promise<void>

/**
 * Interface for event storage implementations
 */
export interface EventStorage {
  /**
   * Save events for a task
   */
  saveEvents(taskId: string, events: Event[]): Promise<void>

  /**
   * Get events for a task
   */
  getEvents(taskId: string): Promise<Event[]>

  /**
   * Delete events for a task
   */
  deleteEvents(taskId: string): Promise<void>
}

/**
 * Interface for database event repository
 */
export interface EventRepository {
  /**
   * Save an event (already delivered)
   */
  saveEvent(event: Event): Promise<void>

  /**
   * Save an event as pending (not yet delivered)
   */
  savePendingEvent(event: Event): Promise<void>

  /**
   * Get events for a task
   */
  getEventsForTask(taskId: string): Promise<Event[]>

  /**
   * Delete events for a task
   */
  deleteEventsForTask(taskId: string): Promise<void>

  /**
   * Mark an event as delivered
   */
  markAsDelivered(eventId: string): Promise<void>

  /**
   * Get an event by ID
   */
  getEventById(eventId: string): Promise<Event | null>

  /**
   * Get pending events (not yet delivered)
   */
  getPendingEvents(): Promise<Event[]>
}

/**
 * Transport for cross-process event delivery
 */
export interface EventTransport {
  /**
   * Initialize the transport (establish connections, subscribe to queues)
   */
  initialize(): Promise<void>

  /**
   * Publish an event to the transport
   * @returns Promise that resolves when event is published (not delivered)
   */
  publishEvent(event: Event): Promise<void>

  /**
   * Start listening for events (for consumers)
   * @param handler - Callback invoked when event is received
   */
  startListening(handler: (event: Event) => Promise<void>): Promise<void>

  /**
   * Stop listening for events
   */
  stopListening(): Promise<void>

  /**
   * Health check
   */
  isHealthy(): boolean

  /**
   * Cleanup connections
   */
  close(): Promise<void>
}

/**
 * Configuration options for BaseEventService
 */
export interface EventServiceOptions {
  /**
   * Maximum number of listeners
   */
  maxListeners?: number

  /**
   * TTL for deduplication cache (ms)
   */
  deduplicationTtlMs?: number

  /**
   * How often to clean up expired events
   */
  cleanupInterval?: number

  /**
   * Optional logger
   */
  logger?: EventLogger
}

/**
 * Logger interface for event service
 */
export interface EventLogger {
  debug(obj: object, msg?: string): void
  info(obj: object, msg?: string): void
  warn(obj: object, msg?: string): void
  error(obj: object, msg?: string): void
}

/**
 * Event query options
 */
export interface EventQueryOptions {
  limit?: number
  offset?: number
  eventType?: EventType
}
