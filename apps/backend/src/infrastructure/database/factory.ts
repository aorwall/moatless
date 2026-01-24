import type { Database } from './database'
import { createPostgresDatabase } from './postgres'
import type { DatabaseConfig } from './types'

/**
 * Create a PostgreSQL database instance.
 *
 * @param config - Database configuration
 * @returns Initialized database instance
 * @throws Error if required configuration is missing
 */
export function createDatabase(config: DatabaseConfig): Database {
  if (!config.postgresUrl) {
    throw new Error('DATABASE_URL is required for PostgreSQL connection')
  }
  return createPostgresDatabase(config.postgresUrl)
}

/**
 * Get database configuration from environment variables.
 *
 * Environment variables:
 * - DATABASE_URL: PostgreSQL connection string (required)
 *
 * @returns Database configuration
 */
export function getDatabaseConfig(): DatabaseConfig {
  const postgresUrl = process.env['DATABASE_URL']

  if (!postgresUrl) {
    throw new Error('DATABASE_URL environment variable is required')
  }

  return { postgresUrl }
}
