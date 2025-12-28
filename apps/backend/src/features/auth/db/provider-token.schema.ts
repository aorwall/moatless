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
    provider: text('provider').notNull(), // 'github', 'gitlab', 'claude', etc.

    // Authentication method
    authMethod: text('auth_method').notNull(), // 'oauth' or 'pat'

    // Token data
    accessToken: text('access_token').notNull(),
    refreshToken: text('refresh_token'), // OAuth refresh token (for providers that support it)
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

/**
 * Domain type for provider token data used in the repository layer.
 */
export interface ProviderTokenDomain {
  userId: string
  provider: string
  authMethod: 'oauth' | 'pat'
  accessToken: string
  refreshToken: string | null
  tokenType: string | null
  scope: string | null
  providerUserId: string | null
  providerLogin: string | null
  connectedAt: Date
  expiresAt: Date | null
}

/**
 * Convert domain token data to SQLite insert format.
 * SQLite stores timestamps as Unix epoch integers via Drizzle's mode: 'timestamp'.
 */
export function tokenToInsert(token: ProviderTokenDomain): ProviderTokenInsert {
  return {
    userId: token.userId,
    provider: token.provider,
    authMethod: token.authMethod,
    accessToken: token.accessToken,
    refreshToken: token.refreshToken,
    tokenType: token.tokenType,
    scope: token.scope,
    providerUserId: token.providerUserId,
    providerLogin: token.providerLogin,
    connectedAt: token.connectedAt,
    expiresAt: token.expiresAt,
  }
}

/**
 * Convert SQLite select result to domain format.
 */
export function selectToToken(row: ProviderTokenSelect): ProviderTokenDomain {
  return {
    userId: row.userId,
    provider: row.provider,
    authMethod: row.authMethod as 'oauth' | 'pat',
    accessToken: row.accessToken,
    refreshToken: row.refreshToken,
    tokenType: row.tokenType,
    scope: row.scope,
    providerUserId: row.providerUserId,
    providerLogin: row.providerLogin,
    connectedAt: row.connectedAt,
    expiresAt: row.expiresAt,
  }
}
