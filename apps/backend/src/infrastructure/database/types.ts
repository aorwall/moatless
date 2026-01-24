import type { PostgresJsDatabase } from 'drizzle-orm/postgres-js'

/**
 * Drizzle database type for PostgreSQL.
 */
export type DrizzleDatabase = PostgresJsDatabase

/**
 * Configuration for database connection.
 */
export interface DatabaseConfig {
  /** PostgreSQL connection URL */
  postgresUrl: string
}
