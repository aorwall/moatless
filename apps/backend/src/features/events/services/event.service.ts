import type { EventLogger, EventTransport } from '@moatless/events'
import { BaseEventService } from '@moatless/events'
import type { Event } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils'
import { eventEmittedTotal } from '../../metrics'

/**
 * EventService - Backend event service with InversifyJS and Prometheus metrics
 *
 * Extends BaseEventService with:
 * - InversifyJS dependency injection
 * - Prometheus metrics tracking
 * - Real-time event streaming (no file-based persistence)
 */
@injectable()
export class EventService extends BaseEventService {
  constructor() {
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

    logger.info('EventService initialized')
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
    eventEmittedTotal.labels({ type: event.type }).inc()

    // Delegate to BaseEventService
    await super.emitEvent(event)
  }

  /**
   * Emit event locally (no re-publish to transport) with metrics tracking
   */
  override async emitLocal(event: Event): Promise<void> {
    eventEmittedTotal.labels({ type: event.type }).inc()
    await super.emitLocal(event)
  }
}
