import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database.js'
import { DatabaseSymbol } from '../../../infrastructure/database/database.js'
import { logger } from '../../../utils/logger.js'
import { eventDeliveryLatencySeconds, eventDeliveryTotal } from '../../metrics'
import { EventRepository } from '../db/event.repository.js'
import { EventService } from './event.service.js'

/**
 * Notification payload from PostgreSQL NOTIFY
 */
interface NotifyPayload {
  eventId: string
  taskId: string
  type: string
  subType?: string
}

/**
 * Listen metadata returned by postgres.js listen()
 */
interface ListenMeta {
  unlisten(): Promise<void>
}

/**
 * Service that listens for PostgreSQL NOTIFY events on the task_events channel.
 * Provides real-time event delivery without polling.
 *
 * When a worker pod inserts an event into the database, the PostgreSQL trigger
 * sends a NOTIFY with the event metadata. This service receives the notification,
 * fetches the full event from the database, and broadcasts it via EventService.
 *
 * Only active when DATABASE_PROVIDER=postgres.
 */
@injectable()
export class NotifyListenerService {
  private listenMeta: ListenMeta | null = null
  private isRunning = false

  constructor(
    @inject(DatabaseSymbol) private readonly db: Database,
    @inject(EventService) private readonly eventService: EventService,
    @inject(EventRepository) private readonly eventRepository: EventRepository,
  ) {}

  /**
   * Start listening for PostgreSQL NOTIFY events.
   * Should be called during application initialization.
   */
  async start(): Promise<void> {
    if (this.db.provider !== 'postgres') {
      logger.debug('NotifyListenerService: Skipping (not PostgreSQL)')
      return
    }

    if (this.isRunning) {
      logger.warn('NotifyListenerService: Already running')
      return
    }

    // Get raw postgres.js client
    const sql = this.db.getRawClient?.()
    if (!sql) {
      logger.error('NotifyListenerService: getRawClient not available on database')
      return
    }

    try {
      // Listen for notifications on task_events channel
      // sql.listen() returns Promise<ListenMeta> with an unlisten() method
      this.listenMeta = await sql.listen('task_events', (payload: string) => {
        // Handle notification in background (don't await to avoid blocking)
        this.handleNotification(payload).catch((error) => {
          logger.error(
            { err: error instanceof Error ? error : new Error(String(error)) },
            'NotifyListenerService: Error in notification handler',
          )
        })
      })

      this.isRunning = true
      logger.info('NotifyListenerService: Started LISTEN on task_events channel')
    } catch (error) {
      logger.error(
        { err: error instanceof Error ? error : new Error(String(error)) },
        'NotifyListenerService: Failed to start LISTEN',
      )
    }
  }

  /**
   * Stop listening for notifications.
   * Should be called during graceful shutdown.
   */
  async stop(): Promise<void> {
    if (!(this.isRunning && this.listenMeta)) {
      return
    }

    try {
      await this.listenMeta.unlisten()
      this.listenMeta = null
      this.isRunning = false
      logger.info('NotifyListenerService: Stopped LISTEN on task_events channel')
    } catch (error) {
      logger.error(
        { err: error instanceof Error ? error : new Error(String(error)) },
        'NotifyListenerService: Error stopping LISTEN',
      )
    }
  }

  /**
   * Handle a notification from PostgreSQL.
   * Fetches the full event and broadcasts it via EventService.
   */
  private async handleNotification(payload: string): Promise<void> {
    const startTime = Date.now()

    try {
      const data = JSON.parse(payload) as NotifyPayload

      logger.debug(
        {
          eventId: data.eventId,
          taskId: data.taskId,
          type: data.type,
        },
        'NotifyListenerService: Received notification',
      )

      // Fetch full event from database
      const event = await this.eventRepository.getEventById(data.eventId)

      if (!event) {
        logger.warn({ eventId: data.eventId }, 'NotifyListenerService: Event not found in database')
        return
      }

      // Broadcast via EventService (handles deduplication and SSE)
      await this.eventService.emitEvent(event)

      // Mark as delivered since we've now broadcast it
      await this.eventRepository.markAsDelivered(data.eventId)

      // Track successful NOTIFY delivery
      eventDeliveryTotal.labels({ mode: 'notify', status: 'success' }).inc()
      eventDeliveryLatencySeconds
        .labels({ mode: 'notify' })
        .observe((Date.now() - startTime) / 1000)

      logger.debug(
        {
          eventId: data.eventId,
          taskId: data.taskId,
          type: data.type,
        },
        'NotifyListenerService: Event delivered',
      )
    } catch (error) {
      // Track failed NOTIFY delivery
      eventDeliveryTotal.labels({ mode: 'notify', status: 'error' }).inc()

      logger.error(
        { err: error instanceof Error ? error : new Error(String(error)), payload },
        'NotifyListenerService: Failed to handle notification',
      )
    }
  }
}
