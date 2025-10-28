import { Database } from 'bun:sqlite'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { BunSQLiteDatabase } from 'drizzle-orm/bun-sqlite'
import { drizzle } from 'drizzle-orm/bun-sqlite'
import { migrate } from 'drizzle-orm/bun-sqlite/migrator'
import type { Logger } from '../../utils/logger'
import type { Database as DatabaseInterface } from './database'

/**
 * Drizzle ORM implementation using Bun's native SQLite.
 * Provides type-safe database access with automatic migrations.
 */
export class DrizzleDatabase implements DatabaseInterface {
  public readonly drizzle: BunSQLiteDatabase
  private readonly sqlite: Database

  constructor(
    dbPath: string,
    private readonly logger: Logger,
  ) {
    // Create SQLite connection using Bun's native Database
    this.sqlite = new Database(dbPath, { create: true })

    // Enable WAL mode for better concurrency
    this.sqlite.exec('PRAGMA journal_mode = WAL')

    // Create Drizzle instance
    this.drizzle = drizzle(this.sqlite)

    this.logger.info(`Database initialized at ${dbPath}`)
  }

  /**
   * Run migrations to update database schema
   */
  // biome-ignore lint/suspicious/useAwait: Interface requires async for consistency
  async migrate(): Promise<void> {
    try {
      // Resolve migrations folder relative to the backend app directory
      // In production (dist/), go up to backend root, then to drizzle/
      // In development (src/), also go up to backend root, then to drizzle/
      const currentDir = import.meta.url ? dirname(fileURLToPath(import.meta.url)) : __dirname
      const migrationsFolder = resolve(currentDir, '../../../drizzle')
      this.logger.info(`Running migrations from ${migrationsFolder}`)

      migrate(this.drizzle, { migrationsFolder })

      this.logger.info('Migrations completed successfully')
    } catch (error) {
      this.logger.error(
        'Migration failed',
        error instanceof Error ? error : new Error(String(error)),
      )
      throw error
    }
  }

  /**
   * Execute a function within a database transaction
   */
  async transaction<T>(fn: (tx: BunSQLiteDatabase) => Promise<T>): Promise<T> {
    return await this.drizzle.transaction(async (tx) => {
      return await fn(tx)
    })
  }

  /**
   * Close the database connection
   */
  // biome-ignore lint/suspicious/useAwait: Interface requires async for consistency
  async close(): Promise<void> {
    this.sqlite.close()
    this.logger.info('Database connection closed')
  }
}

/**
 * Factory function to create a DrizzleDatabase instance.
 * Handles path resolution and initialization.
 *
 * @param dbPath - Path to SQLite database file
 * @param logger - Logger instance for database operations
 * @returns Initialized DrizzleDatabase instance
 */
export function createDrizzleDatabase(dbPath: string, logger: Logger): DrizzleDatabase {
  return new DrizzleDatabase(dbPath, logger)
}
