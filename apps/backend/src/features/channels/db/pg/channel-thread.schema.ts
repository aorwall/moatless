import { sql } from 'drizzle-orm'
import { index, jsonb, pgTable, serial, text, timestamp, uniqueIndex } from 'drizzle-orm/pg-core'
import type {
  ChannelProvider,
  ChannelThread,
  ChannelThreadProperties,
} from '../channel-thread.schema'

/**
 * Channel threads table schema (PostgreSQL).
 * Tracks message threads across external services (email, Slack) linked to tasks.
 */
export const channelThreads = pgTable(
  'integration_threads',
  {
    id: serial('id').primaryKey(),
    taskId: text('task_id').notNull(),
    provider: text('provider').notNull().$type<ChannelProvider>(),
    externalThreadId: text('external_thread_id').notNull(), // Email Message-ID or Slack thread_ts
    properties: jsonb('properties').notNull().$type<ChannelThreadProperties>(),
    lastMessageAt: timestamp('last_message_at').notNull(),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
  },
  (table) => ({
    taskIdIdx: index('integration_threads_task_idx').on(table.taskId),
    providerThreadIdx: uniqueIndex('integration_threads_provider_thread_idx').on(
      table.provider,
      table.externalThreadId,
    ),
  }),
)

export type ChannelThreadInsert = typeof channelThreads.$inferInsert
export type ChannelThreadSelect = typeof channelThreads.$inferSelect

/**
 * Convert domain model to database insert format.
 */
export function channelThreadToInsert(thread: ChannelThread): ChannelThreadInsert {
  return {
    taskId: thread.taskId,
    provider: thread.provider,
    externalThreadId: thread.externalThreadId,
    properties: thread.properties,
    lastMessageAt: thread.lastMessageAt,
    createdAt: thread.createdAt,
  }
}

/**
 * Convert database row to domain model.
 */
export function selectToChannelThread(row: ChannelThreadSelect): ChannelThread {
  return {
    id: row.id,
    taskId: row.taskId,
    provider: row.provider as ChannelProvider,
    externalThreadId: row.externalThreadId,
    properties: row.properties as ChannelThreadProperties,
    lastMessageAt: row.lastMessageAt,
    createdAt: row.createdAt,
  }
}
