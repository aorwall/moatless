import type { UserSession } from '@moatless/types'
import { pgTable, text, timestamp } from 'drizzle-orm/pg-core'

/**
 * Sessions table schema (PostgreSQL).
 * Stores user session information for authentication.
 *
 * Corresponds to the UserSession interface in @moatless/types.
 */
export const sessions = pgTable('sessions', {
  // Primary key - session ID
  id: text('id').primaryKey(),

  // OAuth provider and user identity
  provider: text('provider').notNull(),
  userId: text('user_id').notNull(),
  userLogin: text('user_login').notNull(),
  userEmail: text('user_email'),
  userName: text('user_name'),
  avatarUrl: text('avatar_url'),

  // OAuth tokens (optional for different auth providers)
  accessToken: text('access_token'),
  idToken: text('id_token'),
  tokenType: text('token_type'),
  scope: text('scope'),

  // Optional group ID for group-based access control
  groupId: text('group_id'),

  // Timestamps
  createdAt: timestamp('created_at').notNull(),
  expiresAt: timestamp('expires_at').notNull(),
})

/**
 * Inferred TypeScript type for inserting a session.
 * Generated from the schema definition.
 */
export type SessionInsert = typeof sessions.$inferInsert

/**
 * Inferred TypeScript type for selecting a session from the database.
 * Generated from the schema definition.
 */
export type SessionSelect = typeof sessions.$inferSelect

/**
 * Convert UserSession domain type to database insert type.
 * Handles field name mapping (camelCase to snake_case).
 */
export function sessionToInsert(session: UserSession): SessionInsert {
  return {
    id: session.id,
    provider: session.provider,
    userId: session.userId,
    userLogin: session.userLogin,
    userEmail: session.userEmail,
    userName: session.userName,
    avatarUrl: session.avatarUrl,
    accessToken: session.accessToken,
    idToken: session.idToken,
    tokenType: session.tokenType,
    scope: session.scope,
    groupId: session.groupId,
    createdAt: session.createdAt,
    expiresAt: session.expiresAt,
  }
}

/**
 * Convert database row to UserSession domain type.
 * Handles field name mapping (snake_case to camelCase).
 */
export function selectToSession(row: SessionSelect): UserSession {
  return {
    id: row.id,
    provider: row.provider,
    userId: row.userId,
    userLogin: row.userLogin,
    userEmail: row.userEmail,
    userName: row.userName,
    avatarUrl: row.avatarUrl,
    accessToken: row.accessToken ?? undefined,
    idToken: row.idToken ?? undefined,
    tokenType: row.tokenType ?? undefined,
    scope: row.scope ?? undefined,
    groupId: row.groupId ?? undefined,
    createdAt: row.createdAt,
    expiresAt: row.expiresAt,
  }
}
