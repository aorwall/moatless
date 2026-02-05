import type { SQL } from 'drizzle-orm'
import { and, eq, isNull, or } from 'drizzle-orm'
import type { PgColumn } from 'drizzle-orm/pg-core'
import { SessionContext } from '../../features/auth/services/session-context'
import { ForbiddenError } from '../../features/auth/utils/session-helpers'

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
 * Options for visibility scope filtering
 */
export interface VisibilityScopeOptions {
  /**
   * Additional WHERE conditions to combine with visibility scope filter
   */
  additionalConditions?: SQL[]
}

/**
 * Check if current user is admin (via session role or ADMIN_USER_IDS env var).
 * Used for admin bypass in visibility scope filtering.
 */
function isCurrentUserAdmin(): boolean {
  const session = SessionContext.getCurrentSession()
  if (!session) {
    return false
  }

  // Check session role
  if (session.role === 'admin') {
    return true
  }

  // Check env-based admin list (for bootstrapping)
  const envAdmins = process.env['ADMIN_USER_IDS']?.split(',').map((id) => id.trim()) ?? []
  return envAdmins.includes(session.userId)
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
export function withUserScope<T extends { createdBy: PgColumn }>(
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

/**
 * Build access control WHERE clause for visibility-scoped queries (repositories, tasks, etc.).
 *
 * This is the unified access control pattern for all entities with visibility fields.
 *
 * Behavior:
 * - Dev mode (AUTH_PROVIDER=disabled): No filtering, returns all records
 * - Production mode: Filters to show:
 *   - Private records created by the current user (visibility='private' AND createdBy=userId)
 *   - All public records (visibility='public')
 *   - Legacy records without an owner (createdBy IS NULL) - treated as public
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
 *   .where(withVisibilityScope(repositories))
 *
 * // With additional conditions
 * const tasks = await db.select()
 *   .from(tasks)
 *   .where(withVisibilityScope(tasks, {
 *     additionalConditions: [eq(tasks.status, 'active')]
 *   }))
 * ```
 */
export function withVisibilityScope<T extends { createdBy: PgColumn; visibility: PgColumn }>(
  table: T,
  options: VisibilityScopeOptions = {},
): SQL | undefined {
  const { additionalConditions } = options

  // In dev mode (AUTH_PROVIDER=disabled), skip all access control
  // This ensures backward compatibility and allows access to all records
  const isDevMode = process.env['AUTH_PROVIDER'] === 'disabled'
  if (isDevMode) {
    if (additionalConditions && additionalConditions.length > 0) {
      return and(...additionalConditions)
    }
    return undefined
  }

  // Check if admin read-all is enabled and user is admin
  // This allows admin users to read all entities regardless of visibility
  const adminReadAllEnabled = process.env['ADMIN_READ_ALL'] === 'true'
  if (adminReadAllEnabled && isCurrentUserAdmin()) {
    // Admin bypass: only apply additional conditions, no visibility filtering
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

  // Build visibility read access clause:
  // 1. User's private records (visibility='private' AND createdBy=userId)
  // 2. All public records (visibility='public')
  // 3. Legacy records (createdBy IS NULL) - treated as public for backward compatibility
  const visibilityReadClause = or(
    // User's private records
    and(eq(table.visibility, 'private'), eq(table.createdBy, userId)),
    // All public records
    eq(table.visibility, 'public'),
    // Legacy records (null createdBy) - visible to all
    isNull(table.createdBy),
  ) as SQL

  // Combine with additional conditions
  if (additionalConditions && additionalConditions.length > 0) {
    return and(visibilityReadClause, ...additionalConditions)
  }

  return visibilityReadClause
}

/**
 * Entity that can be checked for write permissions
 */
export interface VisibilityEntity {
  createdBy?: string | null | undefined
  visibility?: 'private' | 'public' | undefined
}

/**
 * Assert that the current user has write permission for a visibility-controlled entity.
 *
 * Write permission rules:
 * - Only the creator (owner) of an entity can modify it
 * - Legacy entities (null createdBy) cannot be modified by anyone (read-only)
 * - Public entities are read-only for non-owners
 * - Dev mode: All write operations are allowed
 *
 * @param entity - Entity to check (repository, task, etc.)
 * @param entityType - Type of entity for error message (e.g., 'repository', 'task')
 * @throws ForbiddenError if user doesn't have write permission
 *
 * @example
 * ```typescript
 * const repo = await repositoryRepo.get(id)
 * assertVisibilityWritePermission(repo, 'repository') // Throws if not authorized
 * await repositoryRepo.update(id, data)
 * ```
 */
export function assertVisibilityWritePermission(
  entity: VisibilityEntity,
  entityType = 'entity',
): void {
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

  // Legacy entities (null createdBy) are read-only for everyone
  // This prevents accidental modification of shared/legacy data
  if (!entity.createdBy) {
    throw new ForbiddenError(
      `Cannot modify legacy ${entityType}. Legacy ${entityType}s are read-only.`,
    )
  }

  // Only the creator can modify the entity (regardless of visibility)
  if (entity.createdBy !== userId) {
    const visibilityMsg = entity.visibility === 'public' ? 'public ' : ''
    throw new ForbiddenError(
      `Access denied: You do not have permission to modify this ${visibilityMsg}${entityType}. ` +
        `Only the ${entityType} owner can make changes.`,
    )
  }

  // User is the creator - write permission granted
}
