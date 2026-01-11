import type { User, UserRole } from '@moatless/types'
import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { getSchemasFromEnv, type sqliteSchemas } from '../../../infrastructure/database/schemas'
import { selectToUser as pgSelectToUser, userToInsert as pgUserToInsert } from './pg/user.schema'
import {
  selectToUser as sqliteSelectToUser,
  userToInsert as sqliteUserToInsert,
} from './user.schema'

// Get correct schema based on DATABASE_PROVIDER
// Cast to SQLite schema type for TypeScript - runtime uses correct provider
const isPostgres = process.env['DATABASE_PROVIDER'] === 'postgres'
const schemas = getSchemasFromEnv()
const users = schemas.users as typeof sqliteSchemas.users
const userToInsert = isPostgres ? pgUserToInsert : sqliteUserToInsert
const selectToUser = isPostgres ? pgSelectToUser : sqliteSelectToUser

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

  /**
   * Update a user's role.
   *
   * @param userId - User identifier
   * @param role - New role to assign
   * @returns true if user was found and updated, false if user not found
   */
  async updateRole(userId: string, role: UserRole): Promise<boolean> {
    const result = await this.db.drizzle.update(users).set({ role }).where(eq(users.id, userId))

    // Check if any rows were affected
    // Drizzle returns different result shapes per driver
    const rowsAffected =
      (result as unknown as { rowsAffected?: number }).rowsAffected ??
      (result as unknown as { changes?: number }).changes ??
      0

    return rowsAffected > 0
  }

  /**
   * Find users by login name (case-insensitive).
   * Used for email integration to map {username}@inbox.moatless.ai to a user.
   *
   * @param login - The login name to search for
   * @returns Array of matching users (typically 0 or 1)
   */
  async findByLogin(login: string): Promise<User[]> {
    const rows = await this.db.drizzle.select().from(users).where(eq(users.login, login))
    return rows.map(selectToUser)
  }

  /**
   * Find a user by email address.
   * Used for Slack integration to match Slack user email to system user.
   *
   * @param email - The email address to search for
   * @returns User if found, null otherwise
   */
  async findByEmail(email: string): Promise<User | null> {
    const rows = await this.db.drizzle.select().from(users).where(eq(users.email, email)).limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToUser(row)
  }
}
