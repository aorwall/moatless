import type { Event, EventType } from '@moatless/types'
import { and, eq, inArray, isNull, sql } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database.js'
import { DatabaseSymbol } from '../../../infrastructure/database/database.js'
import {
  getSchemasFromEnv,
  type sqliteSchemas,
} from '../../../infrastructure/database/schemas/index.js'
import { logger } from '../../../utils/logger.js'
import { databaseOperationDurationSeconds, databaseOperationTotal } from '../../metrics'
import {
  eventToInsert as sqliteEventToInsert,
  selectToEvent as sqliteSelectToEvent,
} from './event.schema.js'
import {
  eventToInsert as pgEventToInsert,
  selectToEvent as pgSelectToEvent,
} from './pg/event.schema.js'

// Get correct schema based on DATABASE_PROVIDER
const isPostgres = process.env['DATABASE_PROVIDER'] === 'postgres'
const schemas = getSchemasFromEnv()
const taskEvents = schemas.taskEvents as typeof sqliteSchemas.taskEvents
const selectToEvent = isPostgres ? pgSelectToEvent : sqliteSelectToEvent
const eventToInsert = isPostgres ? pgEventToInsert : sqliteEventToInsert

/**
 * Repository for task events stored in the database.
 * Used by the main API to poll for events written by worker pods
 * when HTTP delivery fails.
 */
@injectable()
export class EventRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Get undelivered events, ordered by creation time.
   * @param limit - Maximum number of events to return
   */
  async getUndeliveredEvents(limit = 100): Promise<Event[]> {
    const timer = databaseOperationDurationSeconds
      .labels({ operation: 'select', table: 'task_events' })
      .startTimer()

    try {
      const rows = await this.db.drizzle
        .select()
        .from(taskEvents)
        .where(isNull(taskEvents.deliveredAt))
        .orderBy(taskEvents.createdAt)
        .limit(limit)

      databaseOperationTotal
        .labels({ operation: 'select', table: 'task_events', status: 'success' })
        .inc()
      return rows.map((row) => selectToEvent(row))
    } catch (error) {
      databaseOperationTotal
        .labels({ operation: 'select', table: 'task_events', status: 'error' })
        .inc()
      throw error
    } finally {
      timer()
    }
  }

  /**
   * Get undelivered events for a specific task.
   * @param taskId - Task ID to filter by
   * @param limit - Maximum number of events to return
   */
  async getUndeliveredEventsForTask(taskId: string, limit = 100): Promise<Event[]> {
    const rows = await this.db.drizzle
      .select()
      .from(taskEvents)
      .where(and(eq(taskEvents.taskId, taskId), isNull(taskEvents.deliveredAt)))
      .orderBy(taskEvents.createdAt)
      .limit(limit)

    return rows.map((row) => selectToEvent(row))
  }

  /**
   * Mark an event as delivered.
   * @param eventId - Event ID to mark as delivered
   */
  async markAsDelivered(eventId: string): Promise<void> {
    const timer = databaseOperationDurationSeconds
      .labels({ operation: 'update', table: 'task_events' })
      .startTimer()

    try {
      await this.db.drizzle
        .update(taskEvents)
        .set({ deliveredAt: new Date() })
        .where(eq(taskEvents.eventId, eventId))
      databaseOperationTotal
        .labels({ operation: 'update', table: 'task_events', status: 'success' })
        .inc()
    } catch (error) {
      databaseOperationTotal
        .labels({ operation: 'update', table: 'task_events', status: 'error' })
        .inc()
      throw error
    } finally {
      timer()
    }
  }

  /**
   * Mark multiple events as delivered.
   * @param eventIds - Event IDs to mark as delivered
   */
  async markManyAsDelivered(eventIds: string[]): Promise<void> {
    if (eventIds.length === 0) {
      return
    }

    const timer = databaseOperationDurationSeconds
      .labels({ operation: 'update', table: 'task_events' })
      .startTimer()

    try {
      await this.db.drizzle
        .update(taskEvents)
        .set({ deliveredAt: new Date() })
        .where(inArray(taskEvents.eventId, eventIds))
      databaseOperationTotal
        .labels({ operation: 'update', table: 'task_events', status: 'success' })
        .inc()
    } catch (error) {
      databaseOperationTotal
        .labels({ operation: 'update', table: 'task_events', status: 'error' })
        .inc()
      throw error
    } finally {
      timer()
    }
  }

  /**
   * Get a single event by its eventId.
   * Used by NotifyListenerService to fetch full event data after NOTIFY.
   * @param eventId - Event ID to fetch
   */
  async getEventById(eventId: string): Promise<Event | null> {
    const timer = databaseOperationDurationSeconds
      .labels({ operation: 'select', table: 'task_events' })
      .startTimer()

    try {
      const rows = await this.db.drizzle
        .select()
        .from(taskEvents)
        .where(eq(taskEvents.eventId, eventId))
        .limit(1)

      databaseOperationTotal
        .labels({ operation: 'select', table: 'task_events', status: 'success' })
        .inc()
      return rows[0] ? selectToEvent(rows[0]) : null
    } catch (error) {
      databaseOperationTotal
        .labels({ operation: 'select', table: 'task_events', status: 'error' })
        .inc()
      throw error
    } finally {
      timer()
    }
  }

  /**
   * Get count of undelivered events.
   * Useful for monitoring.
   */
  async getUndeliveredCount(): Promise<number> {
    const result = await this.db.drizzle
      .select({ count: sql<number>`count(*)` })
      .from(taskEvents)
      .where(isNull(taskEvents.deliveredAt))

    return result[0]?.count ?? 0
  }

  /**
   * Delete old delivered events.
   * @param olderThanHours - Delete events delivered more than this many hours ago
   */
  async cleanupDeliveredEvents(olderThanHours = 24): Promise<number> {
    const cutoff = new Date(Date.now() - olderThanHours * 60 * 60 * 1000)

    const result = await this.db.drizzle
      .delete(taskEvents)
      .where(
        and(sql`${taskEvents.deliveredAt} IS NOT NULL`, sql`${taskEvents.deliveredAt} < ${cutoff}`),
      )
      .returning({ id: taskEvents.id })

    const deletedCount = result.length

    if (deletedCount > 0) {
      logger.info({ deletedCount, olderThanHours }, 'Cleaned up old delivered events from database')
    }

    return deletedCount
  }

  // ============================================================
  // Primary storage methods (used when DATABASE_PROVIDER=postgres)
  // ============================================================

  /**
   * Save an event to the database.
   * Events saved directly are marked as delivered immediately (SSE already broadcasted).
   * Uses upsert to handle duplicate eventIds gracefully.
   * @param event - Event to save
   */
  async saveEvent(event: Event): Promise<void> {
    const timer = databaseOperationDurationSeconds
      .labels({ operation: 'insert', table: 'task_events' })
      .startTimer()
    const now = new Date()
    const insert = eventToInsert(event)

    // Ensure createdAt is set (required by SQLite schema)
    if (!insert.createdAt) {
      insert.createdAt = now
    }
    // Mark as delivered since we're saving directly (SSE emission happens in EventService)
    insert.deliveredAt = now

    try {
      await this.db.drizzle
        .insert(taskEvents)
        .values(insert as typeof taskEvents.$inferInsert)
        .onConflictDoNothing() // Skip if eventId already exists (idempotent)
      databaseOperationTotal
        .labels({ operation: 'insert', table: 'task_events', status: 'success' })
        .inc()
    } catch (error) {
      databaseOperationTotal
        .labels({ operation: 'insert', table: 'task_events', status: 'error' })
        .inc()
      // Log but don't throw - event was already emitted via SSE
      logger.warn(
        { err: error as Error, eventId: event.eventId, taskId: event.taskId },
        'Failed to save event to database (may be duplicate)',
      )
    } finally {
      timer()
    }
  }

  /**
   * Save an event as pending delivery.
   * Used by worker pods so the main API can deliver via NOTIFY/polling.
   * @param event - Event to save
   */
  async savePendingEvent(event: Event): Promise<void> {
    const timer = databaseOperationDurationSeconds
      .labels({ operation: 'insert', table: 'task_events' })
      .startTimer()
    const now = new Date()
    const insert = eventToInsert(event)

    // Ensure createdAt is set (required by SQLite schema)
    if (!insert.createdAt) {
      insert.createdAt = now
    }
    // Keep deliveredAt null so poller/NOTIFY can deliver
    insert.deliveredAt = null

    try {
      await this.db.drizzle
        .insert(taskEvents)
        .values(insert as typeof taskEvents.$inferInsert)
        .onConflictDoNothing() // Skip if eventId already exists (idempotent)
      databaseOperationTotal
        .labels({ operation: 'insert', table: 'task_events', status: 'success' })
        .inc()
    } catch (error) {
      databaseOperationTotal
        .labels({ operation: 'insert', table: 'task_events', status: 'error' })
        .inc()
      // Log but don't throw - delivery will retry via poller/notify
      logger.warn(
        { err: error as Error, eventId: event.eventId, taskId: event.taskId },
        'Failed to save pending event to database (may be duplicate)',
      )
    } finally {
      timer()
    }
  }

  /**
   * Get all events for a task, ordered by creation time (newest last).
   * @param taskId - Task ID to get events for
   * @param limit - Maximum number of events to return
   * @param offset - Number of events to skip
   * @param eventType - Optional event type filter
   */
  async getEventsForTask(
    taskId: string,
    limit?: number,
    offset = 0,
    eventType?: EventType,
  ): Promise<Event[]> {
    // Build conditions array
    const conditions = [eq(taskEvents.taskId, taskId)]
    if (eventType) {
      conditions.push(eq(taskEvents.type, eventType))
    }

    // Build query with conditions
    let query = this.db.drizzle
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
    return rows.map((row) => selectToEvent(row))
  }

  /**
   * Delete all events for a task.
   * @param taskId - Task ID to delete events for
   */
  async deleteEventsForTask(taskId: string): Promise<void> {
    const timer = databaseOperationDurationSeconds
      .labels({ operation: 'delete', table: 'task_events' })
      .startTimer()

    try {
      await this.db.drizzle.delete(taskEvents).where(eq(taskEvents.taskId, taskId))
      databaseOperationTotal
        .labels({ operation: 'delete', table: 'task_events', status: 'success' })
        .inc()
      logger.debug({ taskId }, 'Deleted events for task from database')
    } catch (error) {
      databaseOperationTotal
        .labels({ operation: 'delete', table: 'task_events', status: 'error' })
        .inc()
      throw error
    } finally {
      timer()
    }
  }

  /**
   * Get event count for a task.
   * @param taskId - Task ID to count events for
   * @param eventType - Optional event type filter
   */
  async getEventCount(taskId: string, eventType?: EventType): Promise<number> {
    const conditions = [eq(taskEvents.taskId, taskId)]
    if (eventType) {
      conditions.push(eq(taskEvents.type, eventType))
    }

    const result = await this.db.drizzle
      .select({ count: sql<number>`count(*)` })
      .from(taskEvents)
      .where(conditions.length > 1 ? and(...conditions) : conditions[0])

    return result[0]?.count ?? 0
  }
}
