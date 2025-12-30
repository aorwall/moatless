import type { Event } from '@moatless/types'
import { drizzle } from 'drizzle-orm/postgres-js'
import postgres from 'postgres'
// Import the PostgreSQL schema
import { eventToInsert, taskEvents } from '../features/events/db/pg/event.schema'
import { logger } from './logger'

/**
 * Lightweight database writer for events.
 * Used by worker pods to write events directly to the database
 * when HTTP delivery to the main API fails.
 */
export class EventDatabaseWriter {
  private client: ReturnType<typeof postgres> | null = null
  private db: ReturnType<typeof drizzle> | null = null
  private initialized = false

  constructor(private readonly databaseUrl?: string) {}

  /**
   * Check if database writing is available.
   * Returns false if DATABASE_URL is not configured.
   */
  isAvailable(): boolean {
    const url = this.databaseUrl || process.env['DATABASE_URL']
    return Boolean(url)
  }

  /**
   * Initialize the database connection lazily.
   */
  // biome-ignore lint/suspicious/useAwait: Method returns Promise for interface consistency
  private async ensureConnection(): Promise<boolean> {
    if (this.initialized) {
      return this.db !== null
    }

    this.initialized = true
    const url = this.databaseUrl || process.env['DATABASE_URL']

    if (!url) {
      logger.debug('DATABASE_URL not configured, event database fallback disabled')
      return false
    }

    try {
      // Create postgres client with minimal pool for worker
      this.client = postgres(url, {
        max: 2, // Minimal pool size for worker
        idle_timeout: 20, // Close idle connections after 20s
        connect_timeout: 10, // 10s connection timeout
      })

      this.db = drizzle(this.client)

      logger.info('Event database writer initialized')
      return true
    } catch (error) {
      logger.error({ err: error }, 'Failed to initialize event database connection')
      return false
    }
  }

  /**
   * Write an event to the database.
   * Returns true if successful, false if database is unavailable or write failed.
   */
  async writeEvent(event: Event): Promise<boolean> {
    const connected = await this.ensureConnection()
    if (!(connected && this.db)) {
      return false
    }

    try {
      const insert = eventToInsert(event)
      await this.db.insert(taskEvents).values(insert)

      logger.info(
        {
          taskId: event.taskId,
          eventType: event.type,
          eventId: event.eventId,
        },
        'Event written to database fallback',
      )

      return true
    } catch (error) {
      logger.error(
        {
          err: error as Error,
          taskId: event.taskId,
          eventType: event.type,
        },
        'Failed to write event to database',
      )
      return false
    }
  }

  /**
   * Close the database connection.
   * Should be called during graceful shutdown.
   */
  async close(): Promise<void> {
    if (this.client) {
      try {
        await this.client.end()
        logger.info('Event database writer connection closed')
      } catch (error) {
        logger.error({ err: error }, 'Error closing event database connection')
      } finally {
        this.client = null
        this.db = null
        this.initialized = false
      }
    }
  }
}
