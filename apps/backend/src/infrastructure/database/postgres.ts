import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { PostgresJsDatabase } from 'drizzle-orm/postgres-js'
import { drizzle } from 'drizzle-orm/postgres-js'
import { migrate } from 'drizzle-orm/postgres-js/migrator'
import type { Logger } from 'pino'
import type { Sql } from 'postgres'
import postgres from 'postgres'
import { logger } from '../../utils/logger'
import type { Database } from './database'

/**
 * Drizzle ORM implementation using PostgreSQL via postgres.js.
 * Provides type-safe database access with automatic migrations.
 * Designed for production use with managed PostgreSQL services (Azure, AWS RDS, etc).
 */
export class PostgresDatabase implements Database {
  public readonly drizzle: PostgresJsDatabase
  private readonly client: Sql
  private readonly logger: Logger

  constructor(connectionUrl: string) {
    // Determine SSL mode based on connection URL
    // Azure Database for PostgreSQL requires SSL
    const requireSsl =
      connectionUrl.includes('azure') ||
      connectionUrl.includes('postgres.database.azure.com') ||
      connectionUrl.includes('sslmode=require')

    this.client = postgres(connectionUrl, {
      max: 10, // Connection pool size
      ssl: requireSsl ? 'require' : false,
      idle_timeout: 0, // Disable idle timeout to prevent negative timeout warnings
      max_lifetime: 60 * 30, // 30 minutes - handles connection cleanup
      connect_timeout: 30, // 30 second timeout for initial connection
      onnotice: (notice) => {
        // Format PostgreSQL NOTICE messages through pino logger
        // These are typically informational messages during migrations
        logger.debug({ pgNotice: notice['message'], code: notice['code'] }, 'PostgreSQL notice')
      },
    })

    this.drizzle = drizzle(this.client)
    this.logger = logger
  }

  /**
   * Run migrations to update database schema.
   * Uses PostgreSQL-specific migrations from drizzle-postgres folder.
   */
  async migrate(): Promise<void> {
    try {
      const migrationsFolder = this.resolveMigrationsFolder()

      this.logger.debug('=== PostgreSQL Migration Start ===')
      this.logger.debug({ migrationsFolder }, 'Migrations folder')

      // List migration files
      const fs = await import('node:fs')
      try {
        const files = fs
          .readdirSync(migrationsFolder)
          .filter((f) => f.endsWith('.sql'))
          .sort()
        this.logger.debug({ count: files.length, files }, 'Found migration files')
      } catch (fsError) {
        this.logger.error(
          { err: fsError instanceof Error ? fsError : new Error(String(fsError)) },
          'Failed to read migrations folder',
        )
        throw fsError
      }

      // Run migrations using the PostgreSQL drizzle instance
      this.logger.debug('Applying pending migrations...')
      await migrate(this.drizzle, { migrationsFolder })

      this.logger.info('PostgreSQL migrations completed')
    } catch (error) {
      this.logger.error(
        { err: error instanceof Error ? error : new Error(String(error)) },
        'PostgreSQL migration failed',
      )
      throw error
    }
  }

  /**
   * Execute a function within a database transaction.
   */
  async transaction<T>(fn: (tx: PostgresJsDatabase) => Promise<T>): Promise<T> {
    return await this.drizzle.transaction(async (tx) => {
      return await fn(tx)
    })
  }

  /**
   * Close the database connection pool.
   */
  async close(): Promise<void> {
    await this.client.end()
    this.logger.info('PostgreSQL connection closed')
  }

  /**
   * Get the raw postgres.js client for LISTEN/NOTIFY operations.
   * Used by NotifyListenerService for real-time event delivery.
   */
  getRawClient(): Sql {
    return this.client
  }

  /**
   * Resolve the migrations folder path.
   * Supports both development and production builds.
   */
  private resolveMigrationsFolder(): string {
    const currentDir = import.meta.url ? dirname(fileURLToPath(import.meta.url)) : __dirname

    // Try multiple possible locations for PostgreSQL migrations folder:
    // 1. Production (bundled): ../drizzle-postgres from dist/server.js
    // 2. Development: ../../../drizzle-postgres from apps/backend/src/infrastructure/database/
    const possiblePaths = [
      resolve(currentDir, '../drizzle-postgres'),
      resolve(currentDir, '../../../drizzle-postgres'),
    ]

    const fs = require('node:fs')
    for (const path of possiblePaths) {
      if (fs.existsSync(path)) {
        return path
      }
    }

    throw new Error(`PostgreSQL migrations folder not found. Checked: ${possiblePaths.join(', ')}`)
  }
}

/**
 * Factory function to create a PostgresDatabase instance.
 *
 * @param connectionUrl - PostgreSQL connection URL
 * @returns Initialized PostgresDatabase instance
 */
export function createPostgresDatabase(connectionUrl: string): PostgresDatabase {
  return new PostgresDatabase(connectionUrl)
}
