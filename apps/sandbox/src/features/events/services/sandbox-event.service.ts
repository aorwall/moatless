import type { EventLogger, EventRepository } from '@moatless/events'
import { BaseEventService } from '@moatless/events'
import { logger } from '../../../shared/logger'
import { SandboxEventRepository } from '../db/sandbox-event.repository'

/**
 * Sandbox event service that extends BaseEventService.
 *
 * Key differences from backend EventService:
 * - No InversifyJS decorators
 * - No Prometheus metrics
 * - WORKER_MODE: always saves events as pending (markDeliveredOnSave: false)
 * - Uses sandbox logger instead of backend logger
 *
 * Events flow:
 * 1. Sandbox emits event via emitEvent()
 * 2. Event saved to PostgreSQL with status='pending'
 * 3. PostgreSQL trigger fires pg_notify()
 * 4. Backend NotifyListenerService receives notification
 * 5. Backend fetches event and broadcasts to SSE clients
 * 6. Backend marks event as delivered
 */
export class SandboxEventService extends BaseEventService {
  private eventRepository?: EventRepository

  constructor(databaseUrl?: string) {
    // Create event logger adapter for BaseEventService
    const eventLogger: EventLogger = {
      debug: (obj: object, msg?: string) => logger.debug(obj, msg),
      info: (obj: object, msg?: string) => logger.info(obj, msg),
      warn: (obj: object, msg?: string) => logger.warn(obj, msg),
      error: (obj: object, msg?: string) => logger.error(obj, msg),
    }

    // Initialize BaseEventService
    super({
      useDatabase: !!databaseUrl,
      markDeliveredOnSave: false, // WORKER_MODE: save as pending
      logger: eventLogger,
      maxListeners: 50,
      deduplicationTtlMs: 60000, // 60s
      cleanupInterval: 100,
    })

    // Initialize repository if database URL provided
    if (databaseUrl) {
      this.eventRepository = new SandboxEventRepository(databaseUrl)
      this.setRepository(this.eventRepository)

      logger.info({ mode: 'postgres', workerMode: true }, 'Sandbox event service initialized')
    } else {
      logger.info({ mode: 'memory-only', workerMode: true }, 'Sandbox event service initialized')
    }
  }

  /**
   * Close database connection on shutdown
   */
  async close(): Promise<void> {
    if (this.eventRepository instanceof SandboxEventRepository) {
      await this.eventRepository.close()
    }
  }
}

/**
 * Factory function to create SandboxEventService instance
 */
export function createSandboxEventService(databaseUrl?: string): SandboxEventService {
  return new SandboxEventService(databaseUrl)
}
