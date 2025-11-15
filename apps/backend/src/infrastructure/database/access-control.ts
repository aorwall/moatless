import type { SQL } from 'drizzle-orm'
import { and, eq, isNull, or } from 'drizzle-orm'
import type { SQLiteColumn } from 'drizzle-orm/sqlite-core'
import { SessionContext } from '../../features/auth/services/session-context'

/**
 * Options for user scope filtering
 */
export interface UserScopeOptions {
  /**
   * Additional WHERE conditions to combine with user scope filter
   */
  additionalConditions?: SQL[]

  /**
   * If true, only return records owned by the user (exclude null createdBy)
   * Default: false (include records with null createdBy)
   */
  strictUserOnly?: boolean
}

/**
 * Build access control WHERE clause for user-scoped queries.
 *
 * Behavior:
 * - Dev mode (AUTH_PROVIDER=disabled): No filtering, returns all records
 * - Production mode: Filters to show only:
 *   - Records created by the current user (createdBy = userId)
 *   - Records without an owner (createdBy IS NULL) - legacy/shared records
 *
 * @param table - Drizzle table with createdBy column
 * @param options - Additional filtering options
 * @returns SQL WHERE clause or undefined if no filtering needed (dev mode only)
 *
 * @example
 * ```typescript
 * // Simple usage
 * const rows = await db.select()
 *   .from(tasks)
 *   .where(withUserScope(tasks))
 *
 * // With additional conditions
 * const rows = await db.select()
 *   .from(tasks)
 *   .where(withUserScope(tasks, {
 *     additionalConditions: [eq(tasks.status, 'active')]
 *   }))
 * ```
 */
export function withUserScope<T extends { createdBy: SQLiteColumn }>(
  table: T,
  options: UserScopeOptions = {},
): SQL | undefined {
  const { additionalConditions, strictUserOnly = false } = options

  // In dev mode (AUTH_PROVIDER=disabled), skip all access control
  // This ensures backward compatibility and allows access to all records
  const isDevMode = process.env['AUTH_PROVIDER'] === 'disabled'
  if (isDevMode) {
    if (additionalConditions && additionalConditions.length > 0) {
      return and(...additionalConditions)
    }
    return undefined
  }

  // Get the authenticated user ID
  // Session middleware ensures this is set for authenticated requests
  const userId = SessionContext.getCurrentUserId()

  // userId should always be present
  // If it's not, this indicates authentication middleware is not working correctly
  if (!userId) {
    throw new Error(
      'Access control error: No authenticated user in production mode. ' +
        'This indicates a configuration or middleware issue.',
    )
  }

  // Build user access clause
  let userAccessClause: SQL

  if (strictUserOnly) {
    // Only show records owned by this user
    userAccessClause = eq(table.createdBy, userId)
  } else {
    // Show user's records + legacy records (null createdBy)
    userAccessClause = or(eq(table.createdBy, userId), isNull(table.createdBy)) as SQL
  }

  // Combine with additional conditions
  if (additionalConditions && additionalConditions.length > 0) {
    return and(userAccessClause, ...additionalConditions)
  }

  return userAccessClause
}
