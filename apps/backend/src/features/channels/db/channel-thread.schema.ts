import { sql } from 'drizzle-orm'
import { index, integer, sqliteTable, text, uniqueIndex } from 'drizzle-orm/sqlite-core'

/**
 * Provider-specific properties for email channel threads.
 */
export interface EmailThreadProperties {
  recipientEmail: string // Who to reply to
  replyFromEmail: string // Email address to send replies from (repository email)
  initialSubject: string // For Re: prefix
  fromEmail: string // Original sender
}

/**
 * Provider-specific properties for Slack channel threads.
 */
export interface SlackThreadProperties {
  channelId: string // Slack channel
  teamId: string // Slack workspace
  userId: string // Slack user who started thread
}

/**
 * Union type for all provider-specific properties.
 */
export type ChannelThreadProperties = EmailThreadProperties | SlackThreadProperties

/**
 * Channel provider types.
 */
export type ChannelProvider = 'email' | 'slack'

/**
 * Channel threads table schema.
 * Tracks message threads across external services (email, Slack) linked to tasks.
 */
export const channelThreads = sqliteTable(
  'integration_threads',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    taskId: text('task_id').notNull(),
    provider: text('provider').notNull().$type<ChannelProvider>(),
    externalThreadId: text('external_thread_id').notNull(), // Email Message-ID or Slack thread_ts
    properties: text('properties', { mode: 'json' }).notNull().$type<ChannelThreadProperties>(),
    lastMessageAt: integer('last_message_at', { mode: 'timestamp' }).notNull(),
    createdAt: integer('created_at', { mode: 'timestamp' })
      .notNull()
      .default(sql`CURRENT_TIMESTAMP`),
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
 * Domain model for channel threads.
 */
export interface ChannelThread {
  id?: number
  taskId: string
  provider: ChannelProvider
  externalThreadId: string
  properties: ChannelThreadProperties
  lastMessageAt: Date
  createdAt: Date
}

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
    provider: row.provider,
    externalThreadId: row.externalThreadId,
    properties: row.properties,
    lastMessageAt: row.lastMessageAt,
    createdAt: row.createdAt,
  }
}
