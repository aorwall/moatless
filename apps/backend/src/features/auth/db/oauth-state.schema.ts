import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * OAuth State Table
 *
 * Stores temporary CSRF state tokens for OAuth flows.
 * Used during the OAuth authorization code flow to prevent CSRF attacks.
 *
 * Lifecycle:
 * 1. Created when user initiates OAuth flow (e.g., linking GitHub account)
 * 2. Validated on OAuth callback to ensure request authenticity
 * 3. Deleted after successful OAuth exchange or expiration (10 minutes)
 */
export const oauthStates = sqliteTable('oauth_states', {
  /**
   * The state token (random CSRF token)
   * Primary key - guaranteed unique, used for lookups
   */
  state: text('state').primaryKey(),

  /**
   * User ID who initiated the OAuth flow
   */
  userId: text('user_id').notNull(),

  /**
   * Provider identifier (e.g., 'github')
   */
  provider: text('provider').notNull(),

  /**
   * When the state was created
   * Stored as Unix timestamp (milliseconds)
   */
  createdAt: integer('created_at', { mode: 'timestamp' }).notNull(),

  /**
   * When the state expires (typically 10 minutes after creation)
   * Stored as Unix timestamp (milliseconds)
   */
  expiresAt: integer('expires_at', { mode: 'timestamp' }).notNull(),
})

/**
 * TypeScript type for OAuth state records
 */
export type OAuthState = typeof oauthStates.$inferSelect

/**
 * TypeScript type for inserting OAuth state records
 */
export type OAuthStateInsert = typeof oauthStates.$inferInsert
