import type { BunSQLiteDatabase } from 'drizzle-orm/bun-sqlite'
import type { ServiceIdentifier } from 'inversify'

/**
 * Database interface for dependency injection.
 * Abstracts database operations to enable testing and provider switching.
 */
export interface Database {
  /**
   * Access to the underlying Drizzle instance for type-safe queries.
   */
  readonly drizzle: BunSQLiteDatabase

  /**
   * Run migrations to update database schema.
   * Should be called during application initialization.
   */
  migrate(): Promise<void>

  /**
   * Execute a function within a database transaction.
   * Automatically commits on success, rolls back on error.
   *
   * @param fn - Function to execute within transaction
   * @returns Result of the transaction function
   */
  transaction<T>(fn: (tx: BunSQLiteDatabase) => Promise<T>): Promise<T>

  /**
   * Close the database connection.
   * Should be called during application shutdown.
   */
  close(): Promise<void>
}

/**
 * InversifyJS symbol for Database interface dependency injection.
 * Use this in @inject() decorators to receive Database instances.
 *
 * @example
 * ```typescript
 * @injectable()
 * class UserRepository {
 *   constructor(@inject(DatabaseSymbol) private readonly db: Database) {}
 * }
 * ```
 */
export const DatabaseSymbol: ServiceIdentifier<Database> = Symbol.for('Database')

// Access control utilities
export type { UserScopeOptions, VisibilityEntity, VisibilityScopeOptions } from './access-control'
export {
  assertVisibilityWritePermission,
  withUserScope,
  withVisibilityScope,
} from './access-control'
