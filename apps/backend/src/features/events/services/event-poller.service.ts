import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { EventRepository } from '../db/event.repository.js'
import { EventService } from './event.service.js'

/**
 * Configuration for the event poller.
 */
interface EventPollerConfig {
  /** Polling interval in milliseconds (default: 2000ms) */
  intervalMs: number
  /** Maximum events to process per poll (default: 100) */
  batchSize: number
  /** Whether the poller is enabled (default: based on DATABASE_PROVIDER) */
  enabled: boolean
}

/**
 * Service that polls the database for events written by worker pods
 * and broadcasts them through the EventService.
 *
 * This is the "receiver" side of the database fallback mechanism.
 * Workers write events to the database when HTTP delivery fails,
 * and this service picks them up and broadcasts to SSE clients.
 */
@injectable()
export class EventPollerService {
  private intervalId: NodeJS.Timeout | null = null
  private isPolling = false
  private config: EventPollerConfig

  constructor(
    @inject(EventRepository) private readonly eventRepository: EventRepository,
    @inject(EventService) private readonly eventService: EventService,
  ) {
    // Default configuration
    this.config = {
      intervalMs: Number(process.env['EVENT_POLLER_INTERVAL_MS']) || 2000,
      batchSize: Number(process.env['EVENT_POLLER_BATCH_SIZE']) || 100,
      // Only enabled when using PostgreSQL (the only DB that supports remote writes)
      enabled: process.env['DATABASE_PROVIDER'] === 'postgres',
    }
  }

  /**
   * Start the event poller.
   * Should be called during application startup.
   */
  start(): void {
    if (!this.config.enabled) {
      logger.debug('Event poller disabled (DATABASE_PROVIDER is not postgres)')
      return
    }

    if (this.intervalId) {
      logger.warn('Event poller already started')
      return
    }

    logger.info(
      {
        intervalMs: this.config.intervalMs,
        batchSize: this.config.batchSize,
      },
      'Starting event poller',
    )

    this.intervalId = setInterval(() => this.poll(), this.config.intervalMs)

    // Run an initial poll immediately
    this.poll()
  }

  /**
   * Stop the event poller.
   * Should be called during graceful shutdown.
   */
  stop(): void {
    if (this.intervalId) {
      clearInterval(this.intervalId)
      this.intervalId = null
      logger.info('Event poller stopped')
    }
  }

  /**
   * Poll for undelivered events and broadcast them.
   */
  private async poll(): Promise<void> {
    // Skip if already polling (prevents overlap if poll takes longer than interval)
    if (this.isPolling) {
      return
    }

    this.isPolling = true

    try {
      const events = await this.eventRepository.getUndeliveredEvents(this.config.batchSize)

      if (events.length === 0) {
        return // No events to process
      }

      logger.debug({ eventCount: events.length }, 'Processing events from database')

      const deliveredEventIds: string[] = []

      for (const event of events) {
        try {
          // Broadcast the event through EventService
          await this.eventService.emitEvent(event)
          deliveredEventIds.push(event.eventId)
        } catch (error) {
          logger.error(
            {
              err: error as Error,
              eventId: event.eventId,
              taskId: event.taskId,
            },
            'Failed to emit event from database',
          )
          // Continue processing other events
        }
      }

      // Mark successfully delivered events
      if (deliveredEventIds.length > 0) {
        await this.eventRepository.markManyAsDelivered(deliveredEventIds)
        logger.info(
          { deliveredCount: deliveredEventIds.length },
          'Events delivered from database fallback',
        )
      }
    } catch (error) {
      logger.error({ err: error as Error }, 'Event poller error')
    } finally {
      this.isPolling = false
    }
  }

  /**
   * Get the current status of the poller.
   */
  getStatus(): { enabled: boolean; running: boolean; config: EventPollerConfig } {
    return {
      enabled: this.config.enabled,
      running: this.intervalId !== null,
      config: this.config,
    }
  }
}
