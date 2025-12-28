import { pgTable, text, timestamp } from 'drizzle-orm/pg-core'

/**
 * API Keys table schema (PostgreSQL).
 * Stores API keys for programmatic access to the API.
 *
 * API keys are stored as SHA-256 hashes for security.
 * The actual key is only shown once when created.
 */
export const apiKeys = pgTable('api_keys', {
  // Primary key - unique identifier for the API key
  id: text('id').primaryKey(),

  // User who owns this API key
  userId: text('user_id').notNull(),

  // Human-readable name for the key (e.g., "E2E Test Key", "CI/CD Key")
  name: text('name').notNull(),

  // SHA-256 hash of the actual API key (key is only shown once on creation)
  keyHash: text('key_hash').notNull(),

  // First 8 characters of the key for identification (e.g., "mvk_abc1...")
  keyPrefix: text('key_prefix').notNull(),

  // JSON array of scopes/permissions (e.g., ["read", "write", "admin"])
  // null means full access
  scopes: text('scopes'),

  // Optional group ID for group-based access control
  groupId: text('group_id'),

  // Timestamps
  lastUsedAt: timestamp('last_used_at'),
  expiresAt: timestamp('expires_at'),
  createdAt: timestamp('created_at').notNull(),
})

/**
 * Inferred TypeScript type for inserting an API key.
 */
export type ApiKeyInsert = typeof apiKeys.$inferInsert

/**
 * Inferred TypeScript type for selecting an API key from the database.
 */
export type ApiKeySelect = typeof apiKeys.$inferSelect

// Re-export domain types from SQLite schema (they are database-agnostic)
export type {
  ApiKey,
  ApiKeyCreateResult,
  ApiKeyPayload,
} from '../api-key.schema'
export { selectToApiKey } from '../api-key.schema'
