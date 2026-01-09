import type { User, UserRole } from '@moatless/types'
import { pgTable, text, timestamp } from 'drizzle-orm/pg-core'

/**
 * Users table schema (PostgreSQL).
 * Stores authenticated user information from various OAuth providers.
 *
 * Corresponds to the User interface in @moatless/types.
 */
export const users = pgTable('users', {
  // Primary key - user ID (string to support both GitHub numeric and Auth0 string IDs)
  id: text('id').primaryKey(),

  // User identity fields
  login: text('login').notNull(),
  email: text('email'),
  name: text('name'),
  avatarUrl: text('avatar_url'),

  // OAuth provider identifier (e.g., 'github', 'auth0')
  provider: text('provider').notNull(),

  // User role (admin or user)
  role: text('role').notNull().$type<UserRole>().default('user'),

  // Timestamps
  createdAt: timestamp('created_at').notNull(),
  lastLoginAt: timestamp('last_login_at').notNull(),
})

/**
 * Inferred TypeScript type for inserting a user.
 * Generated from the schema definition.
 */
export type UserInsert = typeof users.$inferInsert

/**
 * Inferred TypeScript type for selecting a user from the database.
 * Generated from the schema definition.
 */
export type UserSelect = typeof users.$inferSelect

/**
 * Convert User domain type to database insert type.
 * Handles field name mapping (camelCase to snake_case).
 */
export function userToInsert(user: User): UserInsert {
  return {
    id: user.id,
    login: user.login,
    email: user.email,
    name: user.name,
    avatarUrl: user.avatarUrl,
    provider: user.provider,
    role: user.role,
    createdAt: user.createdAt,
    lastLoginAt: user.lastLoginAt,
  }
}

/**
 * Convert database row to User domain type.
 * Handles field name mapping (snake_case to camelCase).
 */
export function selectToUser(row: UserSelect): User {
  return {
    id: row.id,
    login: row.login,
    email: row.email,
    name: row.name,
    avatarUrl: row.avatarUrl,
    provider: row.provider,
    role: row.role,
    createdAt: row.createdAt,
    lastLoginAt: row.lastLoginAt,
  }
}
