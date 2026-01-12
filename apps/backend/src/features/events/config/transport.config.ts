import type { EventLogger, EventTransport } from '@moatless/events'
import type { RedisTransportOptions } from '@moatless/events/transports/redis-event-transport'
import { RedisEventTransport } from '@moatless/events/transports/redis-event-transport'

/**
 * Configuration for event transport
 */
export interface TransportConfig {
  redisUrl: string
  consumerGroup?: string
  consumerId?: string
  blockMs?: number
  maxStreamLength?: number
}

/**
 * Load transport configuration from environment variables
 */
export function loadTransportConfig(): TransportConfig {
  const redisUrl = process.env['REDIS_URL'] || 'redis://localhost:6379'

  return {
    redisUrl,
    consumerGroup: process.env['REDIS_CONSUMER_GROUP'] || 'main-app-consumers',
    consumerId:
      process.env['REDIS_CONSUMER_ID'] || `consumer-${process.env['HOSTNAME'] || 'local'}`,
    blockMs: Number(process.env['REDIS_BLOCK_MS']) || 5000,
    maxStreamLength: Number(process.env['REDIS_MAX_STREAM_LENGTH']) || 10000,
  }
}

/**
 * Create and initialize event transport
 */
export async function createEventTransport(
  config: TransportConfig,
  logger: EventLogger,
): Promise<EventTransport> {
  const options: Partial<RedisTransportOptions> = {
    redisUrl: config.redisUrl,
    logger,
  }

  if (config.consumerGroup) {
    options.consumerGroup = config.consumerGroup
  }
  if (config.consumerId) {
    options.consumerId = config.consumerId
  }
  if (config.blockMs) {
    options.blockMs = config.blockMs
  }
  if (config.maxStreamLength) {
    options.maxStreamLength = config.maxStreamLength
  }

  const transport = new RedisEventTransport(options as RedisTransportOptions)

  // Initialize the transport (connect to Redis, create consumer groups)
  await transport.initialize()

  return transport
}
