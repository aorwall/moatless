import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import type { ApiKey, ApiKeyInsert, ApiKeySelect } from './api-key.schema'
import { apiKeys, selectToApiKey } from './api-key.schema'

/**
 * Repository for API key persistence using Drizzle ORM.
 * Provides type-safe database operations for API key entities.
 */
@injectable()
export class ApiKeyRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Create a new API key.
   *
   * @param insert - API key data to insert
   */
  async createApiKey(insert: ApiKeyInsert): Promise<void> {
    await this.db.drizzle.insert(apiKeys).values(insert)
  }

  /**
   * Get an API key by ID.
   *
   * @param keyId - API key identifier
   * @returns API key or null if not found
   */
  async getApiKeyById(keyId: string): Promise<ApiKey | null> {
    const rows = await this.db.drizzle.select().from(apiKeys).where(eq(apiKeys.id, keyId)).limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToApiKey(row)
  }

  /**
   * Get an API key by key hash.
   * Used for key validation.
   *
   * @param keyHash - SHA-256 hash of the API key
   * @returns API key select row or null if not found
   */
  async getApiKeyByHash(keyHash: string): Promise<ApiKeySelect | null> {
    const rows = await this.db.drizzle
      .select()
      .from(apiKeys)
      .where(eq(apiKeys.keyHash, keyHash))
      .limit(1)

    return rows[0] ?? null
  }

  /**
   * Get all API keys for a user.
   *
   * @param userId - User identifier
   * @returns Array of API keys for the user
   */
  async getApiKeysByUserId(userId: string): Promise<ApiKey[]> {
    const rows = await this.db.drizzle.select().from(apiKeys).where(eq(apiKeys.userId, userId))

    return rows.map(selectToApiKey)
  }

  /**
   * Delete an API key by ID.
   *
   * @param keyId - API key identifier
   */
  async deleteApiKey(keyId: string): Promise<void> {
    await this.db.drizzle.delete(apiKeys).where(eq(apiKeys.id, keyId))
  }

  /**
   * Update the last used timestamp for an API key.
   *
   * @param keyId - API key identifier
   */
  async updateLastUsed(keyId: string): Promise<void> {
    await this.db.drizzle
      .update(apiKeys)
      .set({ lastUsedAt: new Date() })
      .where(eq(apiKeys.id, keyId))
  }
}
