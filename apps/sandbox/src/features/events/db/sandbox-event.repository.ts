import type { EventRepository } from '@moatless/events'
import type { Event, EventType } from '@moatless/types'
import { and, eq, isNull } from 'drizzle-orm'
import { drizzle, type PostgresJsDatabase } from 'drizzle-orm/postgres-js'
import postgres from 'postgres'
import { logger } from '../../../shared/logger'
import {
  eventToInsert as pgEventToInsert,
  selectToEvent as pgSelectToEvent,
  taskEvents,
} from './pg/event.schema'

/**
 * Sandbox-specific EventRepository implementation
 * Simplified version without InversifyJS, Prometheus metrics
 * Connects directly to PostgreSQL for event storage
 */
export class SandboxEventRepository implements EventRepository {
  private db: PostgresJsDatabase
  private sql: ReturnType<typeof postgres>
  private logger = logger.child({ service: 'sandbox-event-repository' })

  constructor(databaseUrl: string) {
    this.sql = postgres(databaseUrl)
    this.db = drizzle(this.sql)
  }

  /**
   * Save an event (already delivered)
   * In sandbox mode, this is not typically called since sandbox uses savePendingEvent
   */
  async saveEvent(event: Event): Promise<void> {
    const now = new Date()
    const insert = pgEventToInsert(event)

    if (!insert.createdAt) {
      insert.createdAt = now
    }
    insert.deliveredAt = now

    try {
      await this.db
        .insert(taskEvents)
        .values(insert as typeof taskEvents.$inferInsert)
        .onConflictDoNothing()
    } catch (error) {
      this.logger.warn(
        { err: error as Error, eventId: event.eventId, taskId: event.taskId },
        'Failed to save event to database',
      )
    }
  }

  /**
   * Save an event as pending (not yet delivered)
   * This is the primary method used by sandbox to emit events
   */
  async savePendingEvent(event: Event): Promise<void> {
    const now = new Date()
    const insert = pgEventToInsert(event)

    if (!insert.createdAt) {
      insert.createdAt = now
    }
    insert.deliveredAt = null

    try {
      await this.db
        .insert(taskEvents)
        .values(insert as typeof taskEvents.$inferInsert)
        .onConflictDoNothing()

      this.logger.debug(
        { eventId: event.eventId, taskId: event.taskId, type: event.type },
        'Saved pending event to database',
      )
    } catch (error) {
      this.logger.warn(
        { err: error as Error, eventId: event.eventId, taskId: event.taskId },
        'Failed to save pending event to database',
      )
    }
  }

  /**
   * Get events for a task
   */
  async getEventsForTask(
    taskId: string,
    limit?: number,
    offset = 0,
    eventType?: EventType,
  ): Promise<Event[]> {
    const conditions = [eq(taskEvents.taskId, taskId)]
    if (eventType) {
      conditions.push(eq(taskEvents.type, eventType))
    }

    let query = this.db
      .select()
      .from(taskEvents)
      .where(conditions.length > 1 ? and(...conditions) : conditions[0])
      .orderBy(taskEvents.createdAt)
      .$dynamic()

    if (offset > 0) {
      query = query.offset(offset)
    }
    if (limit !== undefined) {
      query = query.limit(limit)
    }

    const rows = await query
    return rows.map((row) => pgSelectToEvent(row))
  }

  /**
   * Delete events for a task
   */
  async deleteEventsForTask(taskId: string): Promise<void> {
    try {
      await this.db.delete(taskEvents).where(eq(taskEvents.taskId, taskId))
      this.logger.debug({ taskId }, 'Deleted events for task')
    } catch (error) {
      this.logger.error({ err: error as Error, taskId }, 'Failed to delete events for task')
      throw error
    }
  }

  /**
   * Mark an event as delivered
   */
  async markAsDelivered(eventId: string): Promise<void> {
    await this.db
      .update(taskEvents)
      .set({ deliveredAt: new Date() })
      .where(eq(taskEvents.eventId, eventId))
  }

  /**
   * Get event by ID
   */
  async getEventById(eventId: string): Promise<Event | null> {
    const rows = await this.db
      .select()
      .from(taskEvents)
      .where(eq(taskEvents.eventId, eventId))
      .limit(1)

    return rows[0] ? pgSelectToEvent(rows[0]) : null
  }

  /**
   * Get pending events (not yet delivered)
   */
  async getPendingEvents(): Promise<Event[]> {
    const rows = await this.db
      .select()
      .from(taskEvents)
      .where(isNull(taskEvents.deliveredAt))
      .orderBy(taskEvents.createdAt)

    return rows.map((row) => pgSelectToEvent(row))
  }

  /**
   * Close database connection
   */
  async close(): Promise<void> {
    await this.sql.end()
  }
}
