import type { EventLogger, EventTransport } from '@moatless/events'
import { BaseEventService } from '@moatless/events'
import type { Event } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils'
import { eventEmittedTotal } from '../../metrics'
import { EventsStorage } from '../storage/events.storage'

/**
 * EventService - Backend event service with InversifyJS and Prometheus metrics
 *
 * Extends BaseEventService with:
 * - InversifyJS dependency injection
 * - Prometheus metrics tracking
 * - EventsStorage integration for file-based persistence
 */
@injectable()
export class EventService extends BaseEventService {
  private eventsStorage: EventsStorage

  constructor(@inject(EventsStorage) eventsStorage: EventsStorage) {
    // Create event logger adapter for BaseEventService
    const eventLogger: EventLogger = {
      debug: (obj: object, msg?: string) => logger.debug(obj, msg),
      info: (obj: object, msg?: string) => logger.info(obj, msg),
      warn: (obj: object, msg?: string) => logger.warn(obj, msg),
      error: (obj: object, msg?: string) => logger.error(obj, msg),
    }

    // Initialize BaseEventService
    super({
      logger: eventLogger,
      maxListeners: 100,
      deduplicationTtlMs: 60000,
      cleanupInterval: 100,
    })

    // Store reference to events storage
    this.eventsStorage = eventsStorage

    // Set file storage
    this.setStorage(eventsStorage)

    logger.info({ mode: 'file-storage' }, 'EventService initialized')
  }

  /**
   * Set transport for cross-process event delivery
   */
  override setTransport(transport: EventTransport): void {
    super.setTransport(transport)
    logger.info('EventService: Transport configured')
  }

  /**
   * Emit event with Prometheus metrics tracking
   */
  override async emitEvent(event: Event): Promise<void> {
    // Track event emission metrics
    eventEmittedTotal.labels({ type: event.type, storage_mode: 'file' }).inc()

    // Delegate to BaseEventService
    await super.emitEvent(event)
  }

  /**
   * Delete events for a task
   */
  async deleteEvents(taskId: string): Promise<void> {
    await this.eventsStorage.deleteEvents(taskId)
  }
}
