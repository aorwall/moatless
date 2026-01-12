import type { EventLogger, EventTransport } from '@moatless/events'
import { BaseEventService } from '@moatless/events'
import { logger } from '../../../shared/logger'

/**
 * Sandbox event service that extends BaseEventService.
 *
 * Key differences from backend EventService:
 * - No InversifyJS decorators
 * - No Prometheus metrics
 * - Always publishes events to Redis transport
 * - Uses sandbox logger instead of backend logger
 *
 * Events flow:
 * 1. Sandbox emits event via emitEvent()
 * 2. Event saved to file storage
 * 3. Event published to Redis Stream (task-events:{taskId})
 * 4. Backend consumes from Redis Stream
 * 5. Backend broadcasts to SSE clients
 */
export class SandboxEventService extends BaseEventService {
  constructor(transport?: EventTransport) {
    // Create event logger adapter for BaseEventService
    const eventLogger: EventLogger = {
      debug: (obj: object, msg?: string) => logger.debug(obj, msg),
      info: (obj: object, msg?: string) => logger.info(obj, msg),
      warn: (obj: object, msg?: string) => logger.warn(obj, msg),
      error: (obj: object, msg?: string) => logger.error(obj, msg),
    }

    // Initialize BaseEventService (file storage only)
    super({
      logger: eventLogger,
      maxListeners: 50,
      deduplicationTtlMs: 60000,
      cleanupInterval: 100,
    })

    // Set transport if provided
    if (transport) {
      this.setTransport(transport)
    }

    logger.info({ mode: 'file-storage', role: 'publisher' }, 'Sandbox event service initialized')
  }
}

/**
 * Factory function to create SandboxEventService instance with Redis transport
 */
export async function createSandboxEventService(): Promise<SandboxEventService> {
  // Create Redis transport for cross-process communication
  const { RedisEventTransport } = await import('@moatless/events/transports/redis-event-transport')

  const redisUrl = process.env['REDIS_URL'] || 'redis://localhost:6379'

  const transport = new RedisEventTransport({
    redisUrl,
    logger: {
      debug: (obj: object, msg?: string) => logger.debug(obj, msg),
      info: (obj: object, msg?: string) => logger.info(obj, msg),
      warn: (obj: object, msg?: string) => logger.warn(obj, msg),
      error: (obj: object, msg?: string) => logger.error(obj, msg),
    },
  })

  await transport.initialize()

  const service = new SandboxEventService(transport)

  return service
}
