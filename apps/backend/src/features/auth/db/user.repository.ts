import type { User } from '@moatless/types'
import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { selectToUser, users, userToInsert } from './user.schema'

/**
 * Repository for user persistence using Drizzle ORM.
 * Provides type-safe database operations for user entities.
 *
 * This replaces the legacy UserStorage adapter, using a relational database
 * instead of JSON file storage.
 */
@injectable()
export class UserRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a user to the database.
   * Creates a new user or updates an existing one (upsert).
   *
   * @param user - User entity to save
   */
  async saveUser(user: User): Promise<void> {
    const insert = userToInsert(user)

    await this.db.drizzle
      .insert(users)
      .values(insert)
      .onConflictDoUpdate({
        target: users.id,
        set: {
          login: insert.login,
          email: insert.email,
          name: insert.name,
          avatarUrl: insert.avatarUrl,
          lastLoginAt: insert.lastLoginAt,
        },
      })
  }

  /**
   * Get a user by ID.
   *
   * @param userId - User identifier
   * @returns User entity or null if not found
   */
  async getUser(userId: string): Promise<User | null> {
    const rows = await this.db.drizzle.select().from(users).where(eq(users.id, userId)).limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToUser(row)
  }

  /**
   * Delete a user by ID.
   *
   * @param userId - User identifier
   */
  async deleteUser(userId: string): Promise<void> {
    await this.db.drizzle.delete(users).where(eq(users.id, userId))
  }

  /**
   * Get all users.
   * Use with caution - consider pagination for large datasets.
   *
   * @returns Array of all users
   */
  async getAllUsers(): Promise<User[]> {
    const rows = await this.db.drizzle.select().from(users)
    return rows.map(selectToUser)
  }

  /**
   * Get users by provider.
   *
   * @param provider - OAuth provider identifier (e.g., 'github', 'auth0')
   * @returns Array of users for the specified provider
   */
  async getUsersByProvider(provider: string): Promise<User[]> {
    const rows = await this.db.drizzle.select().from(users).where(eq(users.provider, provider))
    return rows.map(selectToUser)
  }
}
