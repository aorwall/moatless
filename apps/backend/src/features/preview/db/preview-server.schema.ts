import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'
import type { PreviewServerRecord } from '../types/registry.js'

/**
 * Preview servers table schema.
 * Stores preview server lifecycle state and connection information.
 *
 * Corresponds to the PreviewServerRecord interface.
 */
export const previewServers = sqliteTable('preview_servers', {
  // Primary key - server ID (same as task ID in current architecture)
  serverId: text('server_id').primaryKey(),

  // Server status - 'stopped' | 'starting' | 'started' | 'failed'
  status: text('status').notNull(),

  // Connection details (optional during initialization)
  port: integer('port'),
  url: text('url'),
  proxyUrl: text('proxy_url'),
  hostname: text('hostname'),

  // Process information
  pid: integer('pid'),

  // Inspector configuration
  inspectorEnabled: integer('inspector_enabled', { mode: 'boolean' }),

  // Error message (if status is 'failed')
  error: text('error'),

  // Additional metadata stored as JSON
  metadata: text('metadata'),

  // Timestamps stored as Unix epoch in milliseconds
  createdAt: integer('created_at', { mode: 'timestamp' }).notNull(),
  updatedAt: integer('updated_at', { mode: 'timestamp' }).notNull(),
})

/**
 * Inferred TypeScript type for inserting a preview server record.
 * Generated from the schema definition.
 */
export type PreviewServerInsert = typeof previewServers.$inferInsert

/**
 * Inferred TypeScript type for selecting a preview server record from the database.
 * Generated from the schema definition.
 */
export type PreviewServerSelect = typeof previewServers.$inferSelect

/**
 * Convert PreviewServerRecord domain type to database insert type.
 * Handles field name mapping and JSON serialization for metadata.
 */
export function recordToInsert(record: PreviewServerRecord): PreviewServerInsert {
  return {
    serverId: record.serverId,
    status: record.status,
    port: record.port,
    url: record.url,
    proxyUrl: record.proxyUrl,
    hostname: record.hostname,
    pid: record.pid,
    inspectorEnabled: record.inspectorEnabled,
    error: record.error,
    metadata: record.metadata ? JSON.stringify(record.metadata) : undefined,
    createdAt: record.createdAt,
    updatedAt: record.updatedAt,
  }
}

/**
 * Convert database row to PreviewServerRecord domain type.
 * Handles field name mapping and JSON deserialization for metadata.
 */
export function selectToRecord(row: PreviewServerSelect): PreviewServerRecord {
  const record: PreviewServerRecord = {
    serverId: row.serverId,
    status: row.status as PreviewServerRecord['status'],
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }

  // Only assign optional properties if they exist (avoids undefined assignments with exactOptionalPropertyTypes)
  if (row.port !== null) {
    record.port = row.port
  }
  if (row.url !== null) {
    record.url = row.url
  }
  if (row.proxyUrl !== null) {
    record.proxyUrl = row.proxyUrl
  }
  if (row.hostname !== null) {
    record.hostname = row.hostname
  }
  if (row.pid !== null) {
    record.pid = row.pid
  }
  if (row.inspectorEnabled !== null) {
    record.inspectorEnabled = row.inspectorEnabled
  }
  if (row.error !== null) {
    record.error = row.error
  }
  if (row.metadata !== null) {
    record.metadata = JSON.parse(row.metadata) as Record<string, unknown>
  }

  return record
}
