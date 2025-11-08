import { eq, lt } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import type { OAuthState, OAuthStateInsert } from './oauth-state.schema'
import { oauthStates } from './oauth-state.schema'

/**
 * Repository for OAuth state persistence using Drizzle ORM.
 * Provides type-safe database operations for temporary OAuth state tokens.
 *
 * OAuth states are short-lived (10 minutes) CSRF protection tokens used during
 * the OAuth authorization code flow. They are created when initiating OAuth,
 * validated on callback, and immediately deleted after use.
 */
@injectable()
export class OAuthStateRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save an OAuth state to the database.
   * Creates a new state entry for CSRF protection during OAuth flows.
   *
   * @param state - The random state token (CSRF protection)
   * @param userId - User ID who initiated the OAuth flow
   * @param provider - Provider identifier (e.g., 'github')
   * @param expiresAt - When the state expires (typically 10 minutes)
   */
  async saveState(state: string, userId: string, provider: string, expiresAt: Date): Promise<void> {
    const insert: OAuthStateInsert = {
      state,
      userId,
      provider,
      createdAt: new Date(),
      expiresAt,
    }

    await this.db.drizzle.insert(oauthStates).values(insert)
  }

  /**
   * Get an OAuth state by state token.
   * Returns null if state doesn't exist or has expired.
   *
   * @param state - The state token to look up
   * @returns OAuth state or null if not found/expired
   */
  async getState(state: string): Promise<OAuthState | null> {
    const rows = await this.db.drizzle
      .select()
      .from(oauthStates)
      .where(eq(oauthStates.state, state))
      .limit(1)

    const oauthState = rows[0]
    if (!oauthState) {
      return null
    }

    // Check if expired
    if (oauthState.expiresAt < new Date()) {
      // Delete expired state and return null
      await this.deleteState(state)
      return null
    }

    return oauthState
  }

  /**
   * Delete an OAuth state by state token.
   * Used to clean up after OAuth callback (success or failure).
   *
   * @param state - The state token to delete
   */
  async deleteState(state: string): Promise<void> {
    await this.db.drizzle.delete(oauthStates).where(eq(oauthStates.state, state))
  }

  /**
   * Delete all expired OAuth states.
   * Efficient batch deletion for cleanup operations.
   *
   * OAuth states expire after 10 minutes. This method removes all
   * states that have passed their expiration time.
   *
   * @returns Number of states deleted
   */
  async deleteExpiredStates(): Promise<number> {
    const now = new Date()
    const result = await this.db.drizzle.delete(oauthStates).where(lt(oauthStates.expiresAt, now))

    // better-sqlite3 returns { changes: number } for delete operations
    return (result as unknown as { changes: number }).changes || 0
  }
}
