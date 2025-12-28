import type { GitHubServiceConfig } from '@moatless/types'
import { and, eq, isNotNull, lte } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { getSchemasFromEnv, type sqliteSchemas } from '../../../infrastructure/database/schemas'
import {
  type ProviderTokenDomain,
  tokenToInsert as pgTokenToInsert,
} from './pg/provider-token.schema'
import {
  type ProviderTokenSelect,
  tokenToInsert as sqliteTokenToInsert,
} from './provider-token.schema'

// Get correct schema based on DATABASE_PROVIDER
// Cast to SQLite schema type for TypeScript - runtime uses correct provider
const isPostgres = process.env['DATABASE_PROVIDER'] === 'postgres'
const schemas = getSchemasFromEnv()
const providerTokens = schemas.providerTokens as typeof sqliteSchemas.providerTokens
const tokenToInsert = isPostgres ? pgTokenToInsert : sqliteTokenToInsert

/**
 * Generic provider token data (not GitHub-specific)
 * Used by providers that need access to all fields including refreshToken
 */
export interface ProviderTokenData {
  userId: string
  provider: string
  authMethod: 'oauth' | 'pat'
  accessToken: string
  refreshToken?: string
  tokenType?: string
  scope?: string
  providerUserId?: string
  providerLogin?: string
  connectedAt: Date
  expiresAt?: Date
}

/**
 * Repository for provider token persistence using Drizzle ORM.
 * Provides type-safe database operations for external provider tokens (OAuth and PAT).
 *
 * This replaces the legacy file-based storage for GitHub service tokens,
 * using a relational database instead of JSON file storage.
 */
@injectable()
export class ProviderTokenRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a provider token to the database.
   * Creates a new token or updates an existing one (upsert by userId + provider).
   *
   * @param userId - User identifier
   * @param provider - Provider identifier (e.g., 'github')
   * @param tokenData - Token data to save
   */
  async saveProviderToken(
    userId: string,
    provider: string,
    tokenData: {
      authMethod: 'oauth' | 'pat'
      accessToken: string
      refreshToken?: string
      tokenType?: string
      scope?: string
      providerUserId?: string
      providerLogin?: string
      connectedAt: Date
      expiresAt?: Date
    },
  ): Promise<void> {
    // Create domain object for conversion
    const domain: ProviderTokenDomain = {
      userId,
      provider,
      authMethod: tokenData.authMethod,
      accessToken: tokenData.accessToken,
      refreshToken: tokenData.refreshToken ?? null,
      tokenType: tokenData.tokenType ?? null,
      scope: tokenData.scope ?? null,
      providerUserId: tokenData.providerUserId ?? null,
      providerLogin: tokenData.providerLogin ?? null,
      connectedAt: tokenData.connectedAt,
      expiresAt: tokenData.expiresAt ?? null,
    }

    // Convert to provider-specific insert format
    const insert = tokenToInsert(domain)

    // SQLite doesn't support composite primary keys directly in onConflictDoUpdate
    // So we use a WHERE clause to achieve upsert behavior
    const existing = await this.getProviderToken(userId, provider)

    if (existing) {
      // Update existing token
      await this.db.drizzle
        .update(providerTokens)
        .set({
          authMethod: insert.authMethod,
          accessToken: insert.accessToken,
          refreshToken: insert.refreshToken,
          tokenType: insert.tokenType,
          scope: insert.scope,
          providerUserId: insert.providerUserId,
          providerLogin: insert.providerLogin,
          connectedAt: insert.connectedAt,
          expiresAt: insert.expiresAt,
        })
        .where(and(eq(providerTokens.userId, userId), eq(providerTokens.provider, provider)))
    } else {
      // Insert new token
      await this.db.drizzle.insert(providerTokens).values(insert)
    }
  }

  /**
   * Get a provider token for a specific user and provider (GitHub-specific format).
   *
   * @param userId - User identifier
   * @param provider - Provider identifier (e.g., 'github')
   * @returns Provider token data or null if not found
   */
  async getProviderToken(userId: string, provider: string): Promise<GitHubServiceConfig | null> {
    const rows = await this.db.drizzle
      .select()
      .from(providerTokens)
      .where(and(eq(providerTokens.userId, userId), eq(providerTokens.provider, provider)))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return this.selectToGitHubConfig(row)
  }

  /**
   * Get a provider token for a specific user and provider (generic format with all fields).
   * Use this for non-GitHub providers that need access to refreshToken and other fields.
   *
   * @param userId - User identifier
   * @param provider - Provider identifier (e.g., 'claude', 'gitlab')
   * @returns Generic provider token data or null if not found
   */
  async getProviderTokenRaw(userId: string, provider: string): Promise<ProviderTokenData | null> {
    const rows = await this.db.drizzle
      .select()
      .from(providerTokens)
      .where(and(eq(providerTokens.userId, userId), eq(providerTokens.provider, provider)))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return {
      userId: row.userId,
      provider: row.provider,
      authMethod: row.authMethod as 'oauth' | 'pat',
      accessToken: row.accessToken,
      connectedAt: row.connectedAt,
      ...(row.refreshToken && { refreshToken: row.refreshToken }),
      ...(row.tokenType && { tokenType: row.tokenType }),
      ...(row.scope && { scope: row.scope }),
      ...(row.providerUserId && { providerUserId: row.providerUserId }),
      ...(row.providerLogin && { providerLogin: row.providerLogin }),
      ...(row.expiresAt && { expiresAt: row.expiresAt }),
    }
  }

  /**
   * Delete a provider token for a specific user and provider.
   *
   * @param userId - User identifier
   * @param provider - Provider identifier (e.g., 'github')
   */
  async deleteProviderToken(userId: string, provider: string): Promise<void> {
    await this.db.drizzle
      .delete(providerTokens)
      .where(and(eq(providerTokens.userId, userId), eq(providerTokens.provider, provider)))
  }

  /**
   * List all provider tokens for a user.
   *
   * @param userId - User identifier
   * @returns Array of provider tokens
   */
  async listProviderTokens(userId: string): Promise<GitHubServiceConfig[]> {
    const rows = await this.db.drizzle
      .select()
      .from(providerTokens)
      .where(eq(providerTokens.userId, userId))

    return rows.map((row) => this.selectToGitHubConfig(row))
  }

  /**
   * Get all tokens for a specific provider that expire before a given time.
   * Used for automatic token refresh.
   *
   * @param provider - Provider identifier (e.g., 'github', 'claude')
   * @param beforeTime - Only return tokens expiring before this time
   * @returns Array of provider tokens expiring before the specified time
   */
  async getTokensExpiringBefore(
    provider: string,
    beforeTime: Date,
  ): Promise<ProviderTokenSelect[]> {
    const rows = await this.db.drizzle
      .select()
      .from(providerTokens)
      .where(
        and(
          eq(providerTokens.provider, provider),
          isNotNull(providerTokens.expiresAt),
          lte(providerTokens.expiresAt, beforeTime),
        ),
      )

    return rows
  }

  /**
   * Convert database row to GitHubServiceConfig type.
   * Handles field name mapping (snake_case to camelCase) and type conversion.
   *
   * Note: Currently returns GitHubServiceConfig for backward compatibility.
   * In the future, this can be generalized to support other providers.
   */
  private selectToGitHubConfig(row: ProviderTokenSelect): GitHubServiceConfig {
    return {
      userId: row.userId,
      authMethod: row.authMethod as 'oauth' | 'pat',
      accessToken: row.accessToken,
      tokenType: row.tokenType ?? undefined,
      scope: row.scope ?? undefined,
      connectedAt: row.connectedAt.toISOString(),
      expiresAt: row.expiresAt?.toISOString(),
      githubUserId: row.providerUserId ?? undefined,
      githubLogin: row.providerLogin ?? undefined,
    }
  }
}
