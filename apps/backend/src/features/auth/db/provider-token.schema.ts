import { integer, primaryKey, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * Provider tokens table schema.
 * Stores OAuth and PAT tokens for external service integration (GitHub, etc.).
 *
 * Used for users who:
 * - Authenticate via a different provider (e.g., Auth0) but need GitHub access
 * - Prefer to use a Personal Access Token instead of OAuth
 * - Need separate tokens for git operations
 *
 * Note: For users who authenticate directly via a provider (e.g., GitHub OAuth login),
 * their session token is used directly. This table is for additional/linked provider tokens.
 */
export const providerTokens = sqliteTable(
  'provider_tokens',
  {
    // Composite primary key: userId + provider
    userId: text('user_id').notNull(),
    provider: text('provider').notNull(), // 'github', 'gitlab', etc.

    // Authentication method
    authMethod: text('auth_method').notNull(), // 'oauth' or 'pat'

    // Token data
    accessToken: text('access_token').notNull(),
    tokenType: text('token_type'), // e.g., 'Bearer'
    scope: text('scope'), // OAuth scopes granted

    // Provider-specific user identification
    providerUserId: text('provider_user_id'), // GitHub user ID, etc.
    providerLogin: text('provider_login'), // GitHub username, etc.

    // Timestamps stored as Unix epoch in milliseconds
    connectedAt: integer('connected_at', { mode: 'timestamp' }).notNull(),
    expiresAt: integer('expires_at', { mode: 'timestamp' }), // OAuth tokens may expire
  },
  (table) => ({
    // Define composite primary key
    pk: primaryKey({ columns: [table.userId, table.provider] }),
  }),
)

/**
 * Inferred TypeScript type for inserting a provider token.
 * Generated from the schema definition.
 */
export type ProviderTokenInsert = typeof providerTokens.$inferInsert

/**
 * Inferred TypeScript type for selecting a provider token from the database.
 * Generated from the schema definition.
 */
export type ProviderTokenSelect = typeof providerTokens.$inferSelect
