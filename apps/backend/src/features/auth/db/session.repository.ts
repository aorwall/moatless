import type { UserSession } from '@moatless/types'
import { eq, lt } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { getSchemasFromEnv, type sqliteSchemas } from '../../../infrastructure/database/schemas'
import {
  selectToSession as pgSelectToSession,
  sessionToInsert as pgSessionToInsert,
} from './pg/session.schema'
import {
  selectToSession as sqliteSelectToSession,
  sessionToInsert as sqliteSessionToInsert,
} from './session.schema'

// Get correct schema based on DATABASE_PROVIDER
// Cast to SQLite schema type for TypeScript - runtime uses correct provider
const isPostgres = process.env['DATABASE_PROVIDER'] === 'postgres'
const schemas = getSchemasFromEnv()
const sessions = schemas.sessions as typeof sqliteSchemas.sessions
const sessionToInsert = isPostgres ? pgSessionToInsert : sqliteSessionToInsert
const selectToSession = isPostgres ? pgSelectToSession : sqliteSelectToSession

/**
 * Repository for session persistence using Drizzle ORM.
 * Provides type-safe database operations for session entities.
 *
 * This replaces the legacy SessionStorage adapter, using a relational database
 * instead of JSON file storage.
 */
@injectable()
export class SessionRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a session to the database.
   * Creates a new session or updates an existing one (upsert).
   *
   * @param session - Session entity to save
   */
  async saveSession(session: UserSession): Promise<void> {
    const insert = sessionToInsert(session)

    await this.db.drizzle
      .insert(sessions)
      .values(insert)
      .onConflictDoUpdate({
        target: sessions.id,
        set: {
          provider: insert.provider,
          userId: insert.userId,
          userLogin: insert.userLogin,
          userEmail: insert.userEmail,
          userName: insert.userName,
          avatarUrl: insert.avatarUrl,
          role: insert.role,
          accessToken: insert.accessToken,
          idToken: insert.idToken,
          tokenType: insert.tokenType,
          scope: insert.scope,
          groupId: insert.groupId,
          expiresAt: insert.expiresAt,
        },
      })
  }

  /**
   * Get a session by ID.
   *
   * @param sessionId - Session identifier
   * @returns Session entity or null if not found
   */
  async getSession(sessionId: string): Promise<UserSession | null> {
    const rows = await this.db.drizzle
      .select()
      .from(sessions)
      .where(eq(sessions.id, sessionId))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToSession(row)
  }

  /**
   * Delete a session by ID.
   *
   * @param sessionId - Session identifier
   */
  async deleteSession(sessionId: string): Promise<void> {
    await this.db.drizzle.delete(sessions).where(eq(sessions.id, sessionId))
  }

  /**
   * Get all expired sessions.
   * Used for cleanup operations.
   *
   * @returns Array of expired sessions
   */
  async getExpiredSessions(): Promise<UserSession[]> {
    const now = new Date()
    const rows = await this.db.drizzle.select().from(sessions).where(lt(sessions.expiresAt, now))

    return rows.map(selectToSession)
  }

  /**
   * Delete all expired sessions.
   * Efficient batch deletion for cleanup operations.
   *
   * @returns Number of sessions deleted
   */
  async deleteExpiredSessions(): Promise<number> {
    const now = new Date()
    const result = await this.db.drizzle.delete(sessions).where(lt(sessions.expiresAt, now))

    // better-sqlite3 returns { changes: number } for delete operations
    return (result as unknown as { changes: number }).changes || 0
  }

  /**
   * Get all sessions.
   * Use with caution - consider pagination for large datasets.
   *
   * @returns Array of all sessions
   */
  async getAllSessions(): Promise<UserSession[]> {
    const rows = await this.db.drizzle.select().from(sessions)
    return rows.map(selectToSession)
  }

  /**
   * Get sessions by user ID.
   *
   * @param userId - User identifier
   * @returns Array of sessions for the specified user
   */
  async getSessionsByUserId(userId: string): Promise<UserSession[]> {
    const rows = await this.db.drizzle.select().from(sessions).where(eq(sessions.userId, userId))
    return rows.map(selectToSession)
  }
}
