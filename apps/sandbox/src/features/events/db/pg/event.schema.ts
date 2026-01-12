import type { Event } from '@moatless/types'
import { sql } from 'drizzle-orm'
import { boolean, index, pgTable, serial, text, timestamp, uniqueIndex } from 'drizzle-orm/pg-core'

/**
 * Task events table schema (PostgreSQL).
 * Stores events emitted by sandbox pods for delivery to main API.
 */
export const taskEvents = pgTable(
  'task_events',
  {
    id: serial('id').primaryKey(),
    eventId: text('event_id').notNull(),
    taskId: text('task_id').notNull(),
    type: text('type').notNull(),
    subType: text('sub_type'),
    payload: text('payload').notNull(),
    isError: boolean('is_error').notNull().default(false),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
    deliveredAt: timestamp('delivered_at'),
  },
  (table) => ({
    eventIdIdx: uniqueIndex('task_events_event_id_idx').on(table.eventId),
    taskIdx: index('task_events_task_idx').on(table.taskId),
    pendingIdx: index('task_events_pending_idx').on(table.deliveredAt),
  }),
)

export type TaskEventInsert = typeof taskEvents.$inferInsert
export type TaskEventSelect = typeof taskEvents.$inferSelect

/**
 * Convert an Event domain type to database insert type.
 */
export function eventToInsert(event: Event): TaskEventInsert {
  const createdAt = event.timestamp
    ? event.timestamp instanceof Date
      ? event.timestamp
      : new Date(event.timestamp)
    : new Date()

  return {
    eventId: event.eventId,
    taskId: event.taskId,
    type: event.type,
    subType: event.subType ?? null,
    payload: JSON.stringify(event),
    isError: event.isError,
    createdAt,
    deliveredAt: null,
  }
}

/**
 * Convert a database row back to an Event domain type.
 */
export function selectToEvent(row: TaskEventSelect): Event {
  return JSON.parse(row.payload) as Event
}
