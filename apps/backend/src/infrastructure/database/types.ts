import type { BunSQLiteDatabase } from 'drizzle-orm/bun-sqlite'
import type { PostgresJsDatabase } from 'drizzle-orm/postgres-js'

/**
 * Supported database providers.
 * - sqlite: Local development with Bun's native SQLite
 * - postgres: Production with PostgreSQL (managed services like Azure Database for PostgreSQL)
 */
export type DatabaseProvider = 'sqlite' | 'postgres'

/**
 * SQLite-specific Drizzle database type.
 * Used as the primary interface type for backwards compatibility.
 */
export type SqliteDrizzleDatabase = BunSQLiteDatabase

/**
 * PostgreSQL-specific Drizzle database type.
 */
export type PostgresDrizzleDatabase = PostgresJsDatabase

/**
 * Generic Drizzle database type that works with both SQLite and PostgreSQL.
 * Note: Due to Drizzle's type system, this union doesn't work well with
 * method calls. The Database interface uses BunSQLiteDatabase for
 * backwards compatibility, with PostgreSQL cast at runtime.
 */
export type DrizzleDatabase = BunSQLiteDatabase | PostgresJsDatabase

/**
 * Configuration for database connection.
 */
export interface DatabaseConfig {
  /** The database provider to use */
  provider: DatabaseProvider
  /** Path to SQLite database file (required for sqlite provider) */
  sqlitePath?: string
  /** PostgreSQL connection URL (required for postgres provider) */
  postgresUrl?: string
}
