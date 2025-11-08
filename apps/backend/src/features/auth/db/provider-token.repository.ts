import type { GitHubServiceConfig } from '@moatless/types'
import { and, eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import {
  type ProviderTokenInsert,
  type ProviderTokenSelect,
  providerTokens,
} from './provider-token.schema'

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
      tokenType?: string
      scope?: string
      providerUserId?: string
      providerLogin?: string
      connectedAt: Date
      expiresAt?: Date
    },
  ): Promise<void> {
    const insert: ProviderTokenInsert = {
      userId,
      provider,
      authMethod: tokenData.authMethod,
      accessToken: tokenData.accessToken,
      tokenType: tokenData.tokenType ?? null,
      scope: tokenData.scope ?? null,
      providerUserId: tokenData.providerUserId ?? null,
      providerLogin: tokenData.providerLogin ?? null,
      connectedAt: tokenData.connectedAt,
      expiresAt: tokenData.expiresAt ?? null,
    }

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
   * Get a provider token for a specific user and provider.
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
