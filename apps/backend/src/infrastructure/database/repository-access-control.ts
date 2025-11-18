import type { SQL } from 'drizzle-orm'
import { and, eq, isNull, or } from 'drizzle-orm'
import type { SQLiteColumn } from 'drizzle-orm/sqlite-core'
import { SessionContext } from '../../features/auth/services/session-context'
import { ForbiddenError } from '../../features/auth/utils/session-helpers'

/**
 * Options for repository read scope filtering
 */
export interface RepositoryReadScopeOptions {
  /**
   * Additional WHERE conditions to combine with repository scope filter
   */
  additionalConditions?: SQL[]
}

/**
 * Build access control WHERE clause for repository read queries.
 *
 * Behavior:
 * - Dev mode (AUTH_PROVIDER=disabled): No filtering, returns all repositories
 * - Production mode: Filters to show:
 *   - Private repositories created by the current user (visibility='private' AND createdBy=userId)
 *   - All public repositories (visibility='public')
 *   - Legacy repositories without an owner (createdBy IS NULL) - treated as public
 *
 * @param table - Drizzle table with createdBy and visibility columns
 * @param options - Additional filtering options
 * @returns SQL WHERE clause or undefined if no filtering needed (dev mode only)
 *
 * @example
 * ```typescript
 * // Simple usage
 * const repos = await db.select()
 *   .from(repositories)
 *   .where(withRepositoryReadScope(repositories))
 *
 * // With additional conditions
 * const repos = await db.select()
 *   .from(repositories)
 *   .where(withRepositoryReadScope(repositories, {
 *     additionalConditions: [eq(repositories.deleted, false)]
 *   }))
 * ```
 */
export function withRepositoryReadScope<
  T extends { createdBy: SQLiteColumn; visibility: SQLiteColumn },
>(table: T, options: RepositoryReadScopeOptions = {}): SQL | undefined {
  const { additionalConditions } = options

  // In dev mode (AUTH_PROVIDER=disabled), skip all access control
  // This ensures backward compatibility and allows access to all repositories
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

  // userId should always be present in production mode
  // If it's not, this indicates authentication middleware is not working correctly
  if (!userId) {
    throw new Error(
      'Access control error: No authenticated user in production mode. ' +
        'This indicates a configuration or middleware issue.',
    )
  }

  // Build repository read access clause:
  // 1. User's private repositories (visibility='private' AND createdBy=userId)
  // 2. All public repositories (visibility='public')
  // 3. Legacy repositories (createdBy IS NULL) - treated as public for backward compatibility
  const repositoryReadClause = or(
    // User's private repositories
    and(eq(table.visibility, 'private'), eq(table.createdBy, userId)),
    // All public repositories
    eq(table.visibility, 'public'),
    // Legacy repositories (null createdBy) - visible to all
    isNull(table.createdBy),
  ) as SQL

  // Combine with additional conditions
  if (additionalConditions && additionalConditions.length > 0) {
    return and(repositoryReadClause, ...additionalConditions)
  }

  return repositoryReadClause
}

/**
 * Repository entity that can be checked for write permissions
 */
export interface RepositoryEntity {
  createdBy?: string | null | undefined
  visibility?: 'private' | 'public' | undefined
}

/**
 * Assert that the current user has write permission for a repository.
 *
 * Write permission rules:
 * - Only the creator (owner) of a repository can modify it
 * - Legacy repositories (null createdBy) cannot be modified by anyone (read-only)
 * - Public repositories are read-only for non-owners
 * - Dev mode: All write operations are allowed
 *
 * @param repository - Repository entity to check
 * @throws ForbiddenError if user doesn't have write permission
 *
 * @example
 * ```typescript
 * const repo = await repositoryRepo.get(id)
 * assertRepositoryWritePermission(repo) // Throws if not authorized
 * await repositoryRepo.update(id, data)
 * ```
 */
export function assertRepositoryWritePermission(repository: RepositoryEntity): void {
  // In dev mode (AUTH_PROVIDER=disabled), allow all write operations
  const isDevMode = process.env['AUTH_PROVIDER'] === 'disabled'
  if (isDevMode) {
    return
  }

  // Get the authenticated user ID
  const userId = SessionContext.getCurrentUserId()

  // userId should always be present in production mode
  if (!userId) {
    throw new Error(
      'Access control error: No authenticated user in production mode. ' +
        'This indicates a configuration or middleware issue.',
    )
  }

  // Legacy repositories (null createdBy) are read-only for everyone
  // This prevents accidental modification of shared/legacy data
  if (!repository.createdBy) {
    throw new ForbiddenError('Cannot modify legacy repository. Legacy repositories are read-only.')
  }

  // Only the creator can modify the repository (regardless of visibility)
  if (repository.createdBy !== userId) {
    const visibilityMsg = repository.visibility === 'public' ? 'public ' : ''
    throw new ForbiddenError(
      `Access denied: You do not have permission to modify this ${visibilityMsg}repository. ` +
        'Only the repository owner can make changes.',
    )
  }

  // User is the creator - write permission granted
}
