import type Redis from 'ioredis'
import { type MessagingLogger, noopLogger } from '../../types.js'

/**
 * Configuration options for Redis client
 */
export interface RedisClientOptions {
  /**
   * Redis connection URL (e.g., redis://localhost:6379)
   */
  redisUrl: string

  /**
   * Logger for client operations
   */
  logger?: MessagingLogger
}

/**
 * Shared Redis client wrapper for messaging transports.
 *
 * Provides connection management and health checking.
 * Can be shared between EventTransport and RequestQueue.
 */
export class RedisMessagingClient {
  private readonly redisUrl: string
  private readonly logger: MessagingLogger
  private redis: Redis | null = null

  constructor(options: RedisClientOptions) {
    this.redisUrl = options.redisUrl
    this.logger = options.logger ?? noopLogger
  }

  /**
   * Initialize the Redis connection
   */
  async initialize(): Promise<void> {
    if (this.redis) {
      this.logger.warn({}, 'RedisMessagingClient: Already initialized')
      return
    }

    try {
      // Dynamic import to avoid requiring ioredis when not used
      const { default: Redis } = await import('ioredis')

      this.redis = new Redis(this.redisUrl, {
        maxRetriesPerRequest: null,
        enableReadyCheck: true,
        lazyConnect: false,
      })

      // Wait for connection
      await new Promise<void>((resolve, reject) => {
        if (!this.redis) {
          return reject(new Error('Redis client not initialized'))
        }

        this.redis.once('ready', () => {
          this.logger.info({ redisUrl: this.redisUrl }, 'RedisMessagingClient: Connected')
          resolve()
        })

        this.redis.once('error', (error) => {
          this.logger.error({ err: error }, 'RedisMessagingClient: Connection error')
          reject(error)
        })
      })
    } catch (error) {
      this.logger.error({ err: error }, 'RedisMessagingClient: Failed to initialize')
      throw error
    }
  }

  /**
   * Get the underlying Redis client
   * @throws Error if not initialized
   */
  getClient(): Redis {
    if (!this.redis) {
      throw new Error('RedisMessagingClient: Not initialized')
    }
    return this.redis
  }

  /**
   * Health check
   */
  isHealthy(): boolean {
    return this.redis?.status === 'ready'
  }

  /**
   * Close the Redis connection
   */
  async close(): Promise<void> {
    if (this.redis) {
      await this.redis.quit()
      this.redis = null
      this.logger.info({}, 'RedisMessagingClient: Connection closed')
    }
  }
}
