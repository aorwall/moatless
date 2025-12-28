import type { Database } from './database'
import { createDrizzleDatabase } from './drizzle'
import { createPostgresDatabase } from './postgres'
import type { DatabaseConfig, DatabaseProvider } from './types'

/**
 * Create a database instance based on the provided configuration.
 * Supports both SQLite (local development) and PostgreSQL (production).
 *
 * @param config - Database configuration
 * @returns Initialized database instance
 * @throws Error if required configuration is missing
 */
export function createDatabase(config: DatabaseConfig): Database {
  switch (config.provider) {
    case 'postgres':
      if (!config.postgresUrl) {
        throw new Error('DATABASE_URL is required for PostgreSQL provider')
      }
      return createPostgresDatabase(config.postgresUrl)

    default:
      // SQLite is the default provider
      if (!config.sqlitePath) {
        throw new Error('Database path is required for SQLite provider')
      }
      return createDrizzleDatabase(config.sqlitePath)
  }
}

/**
 * Get database configuration from environment variables.
 *
 * Environment variables:
 * - DATABASE_PROVIDER: 'sqlite' (default) or 'postgres'
 * - DATABASE_URL: Connection string (SQLite path or PostgreSQL URL)
 *
 * @returns Database configuration
 */
export function getDatabaseConfig(): DatabaseConfig {
  const provider = (process.env['DATABASE_PROVIDER'] || 'sqlite') as DatabaseProvider

  // For SQLite, DATABASE_URL is the file path
  // For PostgreSQL, DATABASE_URL is the connection string
  const databaseUrl = process.env['DATABASE_URL']

  const config: DatabaseConfig = {
    provider,
  }

  if (provider === 'sqlite' && databaseUrl) {
    config.sqlitePath = databaseUrl
  }

  if (provider === 'postgres' && databaseUrl) {
    config.postgresUrl = databaseUrl
  }

  return config
}
