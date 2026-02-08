import type { Event } from '@moatless/types'
import { sql } from 'drizzle-orm'
import { boolean, index, pgTable, serial, text, timestamp, uniqueIndex } from 'drizzle-orm/pg-core'

/**
 * Task events table schema (PostgreSQL).
 * Stores events emitted by sandbox pods for durability.
 *
 * Events are written here as a fallback when HTTP delivery fails,
 * allowing the main API to poll and process them later.
 */
export const taskEvents = pgTable(
  'task_events',
  {
    // Primary key - auto-incrementing
    id: serial('id').primaryKey(),

    // Event identifier from the Event schema (unique)
    eventId: text('event_id').notNull(),

    // Task this event belongs to
    taskId: text('task_id').notNull(),

    // Event type (task, taskTurn, workspace, environment, agent, server, navigation, preview)
    type: text('type').notNull(),

    // Event subtype (e.g., 'started', 'completed', 'error')
    subType: text('sub_type'),

    // Full event payload as JSON text
    payload: text('payload').notNull(),

    // Whether this is an error event
    isError: boolean('is_error').notNull().default(false),

    // When the event was created
    createdAt: timestamp('created_at').notNull().default(sql`now()`),

    // When the event was delivered to main API (null = pending)
    deliveredAt: timestamp('delivered_at'),
  },
  (table) => ({
    // Unique constraint on eventId
    eventIdIdx: uniqueIndex('task_events_event_id_idx').on(table.eventId),
    // Index for querying events by task
    taskIdx: index('task_events_task_idx').on(table.taskId),
    // Index for finding undelivered events efficiently
    pendingIdx: index('task_events_pending_idx').on(table.deliveredAt),
  }),
)

/**
 * Inferred TypeScript type for inserting a task event record.
 */
export type TaskEventInsert = typeof taskEvents.$inferInsert

/**
 * Inferred TypeScript type for selecting a task event record.
 */
export type TaskEventSelect = typeof taskEvents.$inferSelect

/**
 * Convert an Event domain type to database insert type.
 */
export function eventToInsert(event: Event): TaskEventInsert {
  // Ensure timestamp is a Date object (may be string after JSON.parse)
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
    deliveredAt: null, // Initially undelivered
  }
}

/**
 * Convert a database row back to an Event domain type.
 */
export function selectToEvent(row: TaskEventSelect): Event {
  return JSON.parse(row.payload) as Event
}
