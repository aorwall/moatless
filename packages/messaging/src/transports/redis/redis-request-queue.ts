import { randomUUID } from 'node:crypto'
import type Redis from 'ioredis'
import type { PendingMessage, QueueMessage, ReceiveOptions } from '../../queue/queue-types.js'
import type { RequestQueue } from '../../queue/request-queue.js'
import { type MessagingLogger, noopLogger } from '../../types.js'

/**
 * Configuration options for RedisRequestQueue
 */
export interface RedisRequestQueueOptions {
  /**
   * Redis connection URL (e.g., redis://localhost:6379)
   */
  redisUrl: string

  /**
   * Logger for queue operations
   */
  logger?: MessagingLogger

  /**
   * Consumer group name
   * Default: 'request-consumers'
   */
  consumerGroup?: string

  /**
   * Consumer ID (unique identifier for this consumer instance)
   * Default: hostname or random UUID
   */
  consumerId?: string

  /**
   * Default block time for receive in milliseconds
   * Default: 5000 (5 seconds)
   */
  defaultBlockMs?: number

  /**
   * Maximum stream length (MAXLEN ~)
   * Default: 10000
   */
  maxStreamLength?: number
}

/**
 * RedisRequestQueue - Redis Streams implementation of RequestQueue
 *
 * Uses Redis Streams for reliable request queuing with:
 * - Per-key streams (e.g., agent-requests:{taskId})
 * - Consumer groups for at-least-once delivery
 * - Blocking reads for efficient polling
 * - Stream trimming to prevent unbounded growth
 */
export class RedisRequestQueue<T = unknown> implements RequestQueue<T> {
  private readonly redisUrl: string
  private readonly logger: MessagingLogger
  private readonly consumerGroup: string
  private readonly consumerId: string
  private readonly defaultBlockMs: number
  private readonly maxStreamLength: number

  private redis: Redis | null = null
  private initializedGroups = new Set<string>()

  constructor(options: RedisRequestQueueOptions) {
    this.redisUrl = options.redisUrl
    this.logger = options.logger ?? noopLogger
    this.consumerGroup = options.consumerGroup || 'request-consumers'
    this.consumerId =
      options.consumerId ||
      `consumer-${process.env['HOSTNAME'] || Math.random().toString(36).substring(7)}`
    this.defaultBlockMs = options.defaultBlockMs ?? 5000
    this.maxStreamLength = options.maxStreamLength ?? 10000

    this.logger.debug(
      {
        redisUrl: this.redisUrl,
        consumerGroup: this.consumerGroup,
        consumerId: this.consumerId,
      },
      'RedisRequestQueue: Initialized',
    )
  }

  /**
   * Initialize the queue (connect to Redis)
   */
  async initialize(): Promise<void> {
    if (this.redis) {
      this.logger.warn({}, 'RedisRequestQueue: Already initialized')
      return
    }

    try {
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
          this.logger.info({ redisUrl: this.redisUrl }, 'RedisRequestQueue: Connected to Redis')
          resolve()
        })

        this.redis.once('error', (error) => {
          this.logger.error({ err: error }, 'RedisRequestQueue: Connection error')
          reject(error)
        })
      })
    } catch (error) {
      this.logger.error({ err: error }, 'RedisRequestQueue: Failed to initialize')
      throw error
    }
  }

  /**
   * Publish a message to the queue
   */
  async publish(key: string, payload: T): Promise<string> {
    if (!this.redis) {
      throw new Error('RedisRequestQueue: Not initialized')
    }

    const requestId = randomUUID()

    try {
      // Serialize payload to JSON
      const payloadJson = JSON.stringify(payload)

      // Publish to Redis Stream with MAXLEN trimming
      const messageId = await this.redis.xadd(
        key,
        'MAXLEN',
        '~',
        this.maxStreamLength.toString(),
        '*', // Auto-generate message ID
        'requestId',
        requestId,
        'payload',
        payloadJson,
        'timestamp',
        Date.now().toString(),
      )

      this.logger.debug(
        {
          key,
          requestId,
          messageId,
        },
        'RedisRequestQueue: Message published',
      )

      return requestId
    } catch (error) {
      this.logger.error(
        {
          err: error,
          key,
          requestId,
        },
        'RedisRequestQueue: Failed to publish message',
      )
      throw error
    }
  }

  /**
   * Receive a message from the queue
   */
  async receive(key: string, options?: ReceiveOptions): Promise<QueueMessage<T> | null> {
    if (!this.redis) {
      throw new Error('RedisRequestQueue: Not initialized')
    }

    const blockMs = options?.blockMs ?? this.defaultBlockMs
    const count = options?.count ?? 1

    // Ensure consumer group exists
    await this.ensureConsumerGroup(key)

    try {
      // Build XREADGROUP arguments
      const args: (string | number)[] = [
        'GROUP',
        this.consumerGroup,
        this.consumerId,
        'BLOCK',
        blockMs.toString(),
        'COUNT',
        count.toString(),
        'STREAMS',
        key,
        '>', // Read only new messages
      ]

      // @ts-ignore - ioredis xreadgroup accepts variable args but TypeScript can't infer this
      const result = await this.redis.xreadgroup(...args)

      if (!result || result.length === 0) {
        return null
      }

      // Parse the first message from the result
      const [, messages] = result[0] as [string, [string, string[]][]]

      if (!messages || messages.length === 0) {
        return null
      }

      const [messageId, fields] = messages[0]

      // Parse fields array into object
      const data: Record<string, string> = {}
      for (let i = 0; i < fields.length; i += 2) {
        data[fields[i]] = fields[i + 1]
      }

      const payloadStr = data['payload']
      if (!payloadStr) {
        this.logger.warn({ key, messageId }, 'RedisRequestQueue: Missing payload field')
        // ACK invalid message to prevent reprocessing
        await this.acknowledge(key, messageId)
        return null
      }

      const payload = JSON.parse(payloadStr) as T
      const timestamp = data['timestamp']
        ? new Date(Number.parseInt(data['timestamp']))
        : new Date()

      this.logger.debug(
        {
          key,
          messageId,
          requestId: data['requestId'],
        },
        'RedisRequestQueue: Message received',
      )

      return {
        messageId,
        payload,
        timestamp,
      }
    } catch (error) {
      this.logger.error(
        {
          err: error,
          key,
        },
        'RedisRequestQueue: Failed to receive message',
      )
      throw error
    }
  }

  /**
   * Acknowledge a message as processed
   */
  async acknowledge(key: string, messageId: string): Promise<void> {
    if (!this.redis) {
      throw new Error('RedisRequestQueue: Not initialized')
    }

    try {
      await this.redis.xack(key, this.consumerGroup, messageId)

      this.logger.debug(
        {
          key,
          messageId,
        },
        'RedisRequestQueue: Message acknowledged',
      )
    } catch (error) {
      this.logger.error(
        {
          err: error,
          key,
          messageId,
        },
        'RedisRequestQueue: Failed to acknowledge message',
      )
      throw error
    }
  }

  /**
   * List pending (unacknowledged) messages
   */
  async listPending(key: string): Promise<PendingMessage[]> {
    if (!this.redis) {
      throw new Error('RedisRequestQueue: Not initialized')
    }

    try {
      // Ensure consumer group exists first
      await this.ensureConsumerGroup(key)

      // XPENDING returns: [count, minId, maxId, [[consumer, count], ...]]
      const summary = (await this.redis.xpending(key, this.consumerGroup)) as [
        number,
        string | null,
        string | null,
        [string, string][] | null,
      ]

      const count = summary[0]
      if (count === 0) {
        return []
      }

      // Get detailed pending info: XPENDING key group - + count
      const detailed = (await this.redis.xpending(
        key,
        this.consumerGroup,
        '-',
        '+',
        count.toString(),
      )) as [string, string, number, number][]

      return detailed.map(([messageId, consumer, idleTime, deliveryCount]) => ({
        messageId,
        consumer,
        idleTimeMs: idleTime,
        deliveryCount,
      }))
    } catch (error) {
      // If stream doesn't exist, return empty list
      if (error instanceof Error && error.message.includes('NOGROUP')) {
        return []
      }

      this.logger.error(
        {
          err: error,
          key,
        },
        'RedisRequestQueue: Failed to list pending messages',
      )
      throw error
    }
  }

  /**
   * Health check
   */
  isHealthy(): boolean {
    return this.redis?.status === 'ready'
  }

  /**
   * Close Redis connection
   */
  async close(): Promise<void> {
    if (this.redis) {
      await this.redis.quit()
      this.redis = null
      this.initializedGroups.clear()
      this.logger.info({}, 'RedisRequestQueue: Connection closed')
    }
  }

  /**
   * Ensure consumer group exists for a stream
   */
  private async ensureConsumerGroup(key: string): Promise<void> {
    if (!this.redis || this.initializedGroups.has(key)) {
      return
    }

    try {
      // Try to create consumer group (MKSTREAM creates stream if it doesn't exist)
      await this.redis.xgroup('CREATE', key, this.consumerGroup, '0', 'MKSTREAM')
      this.initializedGroups.add(key)

      this.logger.debug({ key, consumerGroup: this.consumerGroup }, 'Consumer group created')
    } catch (error: unknown) {
      // Ignore BUSYGROUP error (group already exists)
      if (error instanceof Error && error.message.includes('BUSYGROUP')) {
        this.initializedGroups.add(key)
        return
      }

      this.logger.error(
        {
          err: error,
          key,
          consumerGroup: this.consumerGroup,
        },
        'RedisRequestQueue: Failed to create consumer group',
      )
    }
  }
}
