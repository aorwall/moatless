import type { Event } from '@moatless/types'
import type Redis from 'ioredis'
import type { EventTransport } from '../../events/event-transport.js'
import { type MessagingLogger, noopLogger } from '../../types.js'

/**
 * Configuration options for RedisEventTransport
 */
export interface RedisEventTransportOptions {
  /**
   * Redis connection URL (e.g., redis://localhost:6379)
   */
  redisUrl: string

  /**
   * Logger for transport operations
   */
  logger?: MessagingLogger

  /**
   * Consumer group name for consumption
   * Default: 'main-app-consumers'
   */
  consumerGroup?: string

  /**
   * Consumer ID (unique identifier for this consumer instance)
   * Default: hostname or random UUID
   */
  consumerId?: string

  /**
   * Block time for XREADGROUP in milliseconds
   * Default: 5000 (5 seconds)
   */
  blockMs?: number

  /**
   * Maximum stream length (MAXLEN ~)
   * Default: 10000
   */
  maxStreamLength?: number
}

/**
 * RedisEventTransport - Redis Streams implementation of EventTransport
 *
 * Uses Redis Streams for reliable cross-process event delivery with:
 * - Per-task ordering: One stream per task (task-events:{taskId})
 * - Consumer groups for at-least-once delivery
 * - Blocking reads for low-latency consumption
 * - Stream trimming to prevent unbounded growth
 */
export class RedisEventTransport implements EventTransport {
  private readonly redisUrl: string
  private readonly logger: MessagingLogger
  private readonly consumerGroup: string
  private readonly consumerId: string
  private readonly blockMs: number
  private readonly maxStreamLength: number

  private redis: Redis | null = null
  private isListening = false
  private stopSignal = false
  private listenerPromise: Promise<void> | null = null

  constructor(options: RedisEventTransportOptions) {
    this.redisUrl = options.redisUrl
    this.logger = options.logger ?? noopLogger
    this.consumerGroup = options.consumerGroup || 'main-app-consumers'
    this.consumerId =
      options.consumerId ||
      `consumer-${process.env['HOSTNAME'] || Math.random().toString(36).substring(7)}`
    this.blockMs = options.blockMs ?? 5000
    this.maxStreamLength = options.maxStreamLength ?? 10000

    this.logger.debug(
      {
        redisUrl: this.redisUrl,
        consumerGroup: this.consumerGroup,
        consumerId: this.consumerId,
      },
      'RedisEventTransport: Initialized',
    )
  }

  /**
   * Initialize the transport (connect to Redis)
   */
  async initialize(): Promise<void> {
    if (this.redis) {
      this.logger.warn({}, 'RedisEventTransport: Already initialized')
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
          this.logger.info({ redisUrl: this.redisUrl }, 'RedisEventTransport: Connected to Redis')
          resolve()
        })

        this.redis.once('error', (error) => {
          this.logger.error({ err: error }, 'RedisEventTransport: Connection error')
          reject(error)
        })
      })
    } catch (error) {
      this.logger.error({ err: error }, 'RedisEventTransport: Failed to initialize')
      throw error
    }
  }

  /**
   * Publish an event to Redis Stream
   */
  async publishEvent(event: Event): Promise<void> {
    if (!this.redis) {
      throw new Error('RedisEventTransport: Not initialized')
    }

    const streamKey = `task-events:${event.taskId}`

    try {
      // Serialize event to JSON
      const eventPayload = JSON.stringify(event)

      // Publish to Redis Stream with MAXLEN trimming
      await this.redis.xadd(
        streamKey,
        'MAXLEN',
        '~',
        this.maxStreamLength.toString(),
        '*', // Auto-generate message ID
        'eventId',
        event.eventId,
        'type',
        event.type,
        'payload',
        eventPayload,
      )

      this.logger.debug(
        {
          streamKey,
          eventId: event.eventId,
          type: event.type,
          taskId: event.taskId,
        },
        'RedisEventTransport: Event published',
      )
    } catch (error) {
      this.logger.error(
        {
          err: error,
          streamKey,
          eventId: event.eventId,
        },
        'RedisEventTransport: Failed to publish event',
      )
      throw error
    }
  }

  /**
   * Start listening for events from Redis Streams
   */
  async startListening(handler: (event: Event) => Promise<void>): Promise<void> {
    if (!this.redis) {
      throw new Error('RedisEventTransport: Not initialized')
    }

    if (this.isListening) {
      this.logger.warn({}, 'RedisEventTransport: Already listening')
      return
    }

    this.isListening = true
    this.stopSignal = false

    this.logger.info(
      {
        consumerGroup: this.consumerGroup,
        consumerId: this.consumerId,
      },
      'RedisEventTransport: Started listening',
    )

    // Start the blocking read loop in the background
    this.listenerPromise = this.consumeLoop(handler)

    // Await to satisfy async signature (required by EventTransport interface)
    await Promise.resolve()
  }

  /**
   * Stop listening for events
   */
  async stopListening(): Promise<void> {
    if (!this.isListening) {
      return
    }

    this.logger.info({}, 'RedisEventTransport: Stopping listener')

    // Signal the consume loop to stop
    this.stopSignal = true
    this.isListening = false

    // Wait for the consume loop to finish
    if (this.listenerPromise) {
      await this.listenerPromise
      this.listenerPromise = null
    }

    this.logger.info({}, 'RedisEventTransport: Listener stopped')
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
    await this.stopListening()

    if (this.redis) {
      await this.redis.quit()
      this.redis = null
      this.logger.info({}, 'RedisEventTransport: Connection closed')
    }
  }

  /**
   * Process all messages from stream results
   */
  private async processStreamResults(
    result: [string, [string, string[]][]][],
    handler: (event: Event) => Promise<void>,
  ): Promise<void> {
    for (const [streamKey, messages] of result) {
      for (const [messageId, fields] of messages) {
        try {
          await this.processMessage(streamKey, messageId, fields, handler)
        } catch (error) {
          this.logger.error(
            {
              err: error,
              streamKey,
              messageId,
            },
            'RedisEventTransport: Failed to process message',
          )
          // Don't ACK failed messages - they'll be reprocessed
        }
      }
    }
  }

  /**
   * Process streams - get active streams and read messages
   */
  private async processStreams(handler: (event: Event) => Promise<void>): Promise<void> {
    const streams = await this.getActiveStreams()

    if (streams.length === 0) {
      // No streams yet, wait and retry
      await this.sleep(1000)
      return
    }

    // Ensure consumer groups exist for all streams
    for (const streamKey of streams) {
      await this.ensureConsumerGroup(streamKey)
    }

    // Read from all streams using XREADGROUP
    const result = await this.readFromStreams(streams)

    if (result) {
      await this.processStreamResults(result, handler)
    }
  }

  /**
   * Consume loop - blocking read from Redis Streams
   */
  private async consumeLoop(handler: (event: Event) => Promise<void>): Promise<void> {
    while (!this.stopSignal && this.redis) {
      try {
        await this.processStreams(handler)
      } catch (error) {
        if (this.stopSignal) {
          // Expected error when stopping
          break
        }

        this.logger.error({ err: error }, 'RedisEventTransport: Error in consume loop')

        // Backoff before retrying
        await this.sleep(2000)
      }
    }

    this.logger.debug({}, 'RedisEventTransport: Consume loop exited')
  }

  /**
   * Read from multiple streams using XREADGROUP
   */
  private async readFromStreams(
    streamKeys: string[],
  ): Promise<[string, [string, string[]][]][] | null> {
    if (!this.redis) {
      return null
    }

    // Build XREADGROUP arguments
    const args: (string | number)[] = [
      'GROUP',
      this.consumerGroup,
      this.consumerId,
      'BLOCK',
      this.blockMs.toString(),
      'COUNT',
      '10', // Read up to 10 messages at once
      'STREAMS',
      ...streamKeys,
      // Use '>' to read only new messages (not pending)
      ...streamKeys.map(() => '>'),
    ]

    // @ts-ignore - ioredis xreadgroup accepts variable args but TypeScript can't infer this
    const result = await this.redis.xreadgroup(...args)

    return result as [string, [string, string[]][]][] | null
  }

  /**
   * Process a single message from Redis Stream
   */
  private async processMessage(
    streamKey: string,
    messageId: string,
    fields: string[],
    handler: (event: Event) => Promise<void>,
  ): Promise<void> {
    // Parse fields array into object
    const data: Record<string, string> = {}
    for (let i = 0; i < fields.length; i += 2) {
      data[fields[i]] = fields[i + 1]
    }

    const payloadStr = data['payload']
    if (!payloadStr) {
      this.logger.warn({ streamKey, messageId }, 'RedisEventTransport: Missing payload field')
      await this.ackMessage(streamKey, messageId)
      return
    }

    try {
      const event = JSON.parse(payloadStr) as Event

      this.logger.debug(
        {
          streamKey,
          messageId,
          eventId: event.eventId,
          type: event.type,
        },
        'RedisEventTransport: Processing message',
      )

      // Call handler
      await handler(event)

      // Acknowledge message after successful processing
      await this.ackMessage(streamKey, messageId)

      this.logger.debug(
        {
          streamKey,
          messageId,
          eventId: event.eventId,
        },
        'RedisEventTransport: Message processed and acknowledged',
      )
    } catch (error) {
      this.logger.error(
        {
          err: error,
          streamKey,
          messageId,
        },
        'RedisEventTransport: Failed to process message',
      )
      throw error
    }
  }

  /**
   * Acknowledge a message (mark as processed)
   */
  private async ackMessage(streamKey: string, messageId: string): Promise<void> {
    if (!this.redis) {
      return
    }

    try {
      await this.redis.xack(streamKey, this.consumerGroup, messageId)
    } catch (error) {
      this.logger.error(
        {
          err: error,
          streamKey,
          messageId,
        },
        'RedisEventTransport: Failed to acknowledge message',
      )
    }
  }

  /**
   * Ensure consumer group exists for a stream
   */
  private async ensureConsumerGroup(streamKey: string): Promise<void> {
    if (!this.redis) {
      return
    }

    try {
      // Try to create consumer group (idempotent)
      await this.redis.xgroup('CREATE', streamKey, this.consumerGroup, '0', 'MKSTREAM')

      this.logger.debug({ streamKey, consumerGroup: this.consumerGroup }, 'Consumer group created')
    } catch (error: unknown) {
      // Ignore BUSYGROUP error (group already exists)
      if (error instanceof Error && error.message.includes('BUSYGROUP')) {
        return
      }

      this.logger.error(
        {
          err: error,
          streamKey,
          consumerGroup: this.consumerGroup,
        },
        'RedisEventTransport: Failed to create consumer group',
      )
    }
  }

  /**
   * Get all active task event streams
   */
  private async getActiveStreams(): Promise<string[]> {
    if (!this.redis) {
      return []
    }

    try {
      const keys: string[] = []
      let cursor = '0'

      do {
        const result = await this.redis.scan(cursor, 'MATCH', 'task-events:*', 'COUNT', '100')
        cursor = result[0]
        keys.push(...result[1])
      } while (cursor !== '0')

      return keys
    } catch (error) {
      this.logger.error({ err: error }, 'RedisEventTransport: Failed to get active streams')
      return []
    }
  }

  /**
   * Sleep utility
   */
  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
