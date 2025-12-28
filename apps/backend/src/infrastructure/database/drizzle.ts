import { Database } from 'bun:sqlite'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { BunSQLiteDatabase } from 'drizzle-orm/bun-sqlite'
import { drizzle } from 'drizzle-orm/bun-sqlite'
import { migrate } from 'drizzle-orm/bun-sqlite/migrator'
import type { Logger } from 'pino'
import { logger } from '../../utils/logger'
import type { Database as DatabaseInterface } from './database'
import type { DatabaseProvider } from './types'

/**
 * Drizzle ORM implementation using Bun's native SQLite.
 * Provides type-safe database access with automatic migrations.
 */
export class DrizzleDatabase implements DatabaseInterface {
  public readonly drizzle: BunSQLiteDatabase
  public readonly provider: DatabaseProvider = 'sqlite'
  private readonly sqlite: Database
  private readonly logger: Logger

  constructor(dbPath: string) {
    // Create SQLite connection using Bun's native Database
    this.sqlite = new Database(dbPath, { create: true })

    // Enable WAL mode for better concurrency
    this.sqlite.exec('PRAGMA journal_mode = WAL')

    // Create Drizzle instance
    this.drizzle = drizzle(this.sqlite)

    this.logger = logger
  }

  /**
   * Run migrations to update database schema
   */
  // biome-ignore lint/suspicious/useAwait: Interface requires async for consistency
  async migrate(): Promise<void> {
    try {
      const currentDir = import.meta.url ? dirname(fileURLToPath(import.meta.url)) : __dirname

      // Try multiple possible locations for migrations folder:
      // 1. Production (bundled): ../drizzle from dist/server.js
      // 2. Development: ../../../drizzle from apps/backend/src/infrastructure/database/
      const possiblePaths = [
        resolve(currentDir, '../drizzle'),
        resolve(currentDir, '../../../drizzle'),
      ]

      this.logger.debug('=== Database Migration Start ===')
      this.logger.debug({ currentDir }, 'Current directory')
      this.logger.debug({ paths: possiblePaths }, 'Checking possible migration paths')

      // Find the first path that exists
      const fs = await import('node:fs')
      let migrationsFolder: string | null = null
      for (const path of possiblePaths) {
        if (fs.existsSync(path)) {
          migrationsFolder = path
          this.logger.debug({ migrationsFolder }, 'Found migrations folder')
          break
        }
      }

      if (!migrationsFolder) {
        this.logger.error({ paths: possiblePaths }, 'Migration folder not found')
        throw new Error('Migration folder not found')
      }

      // List migration files
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

      // Check current migration status
      try {
        const appliedMigrations = this.sqlite
          .query<{ id: number }, []>('SELECT id FROM __drizzle_migrations ORDER BY id')
          .all()
        this.logger.debug({ count: appliedMigrations.length }, 'Already applied migrations')
      } catch {
        // Table doesn't exist yet, this is the first migration
        this.logger.debug('No migrations applied yet (migration table does not exist)')
      }

      // Run migrations
      this.logger.debug('Applying pending migrations...')
      await migrate(this.drizzle, { migrationsFolder })

      // Report final status
      const finalMigrations = this.sqlite
        .query<{ id: number }, []>('SELECT id FROM __drizzle_migrations ORDER BY id')
        .all()
      this.logger.info({ totalApplied: finalMigrations.length }, '✓ Database migrations completed')
    } catch (error) {
      this.logger.error(
        { err: error instanceof Error ? error : new Error(String(error)) },
        '✗ Migration failed - see details above',
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
 * @returns Initialized DrizzleDatabase instance
 */
export function createDrizzleDatabase(dbPath: string): DrizzleDatabase {
  return new DrizzleDatabase(dbPath)
}
