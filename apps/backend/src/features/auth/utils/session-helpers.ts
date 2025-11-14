import { SessionContext } from '../services/session-context'

/**
 * Error thrown when a user attempts to access or modify a resource they don't own
 */
export class ForbiddenError extends Error {
  constructor(message = 'You do not have permission to access this resource') {
    super(message)
    this.name = 'ForbiddenError'
  }
}

/**
 * Assert that the current user has ownership/access to a resource.
 * Throws ForbiddenError if the user doesn't own the resource.
 *
 * Rules:
 * - User must own the resource (entity.createdBy === userId)
 * - OR resource has no owner (entity.createdBy is null/undefined) - shared/legacy
 * - In dev mode (no userId), all access is allowed
 *
 * @param entity - Entity to check ownership of
 * @param options - Assertion options
 * @throws {ForbiddenError} If user doesn't have access
 *
 * @example
 * ```typescript
 * const task = await taskRepository.get(taskId)
 * if (!task) throw new NotFoundError()
 *
 * // Verify ownership before deletion
 * assertOwnership(task)
 * await taskRepository.delete(taskId)
 * ```
 */
export function assertOwnership(
  entity: { createdBy?: string | null | undefined },
  options: { allowLegacy?: boolean } = {},
): void {
  const { allowLegacy = true } = options
  const userId = SessionContext.getCurrentUserId()

  // In dev mode (no auth), allow all access
  if (!userId) {
    return
  }

  const isOwner = entity.createdBy === userId
  const isLegacy = !entity.createdBy && allowLegacy

  if (!(isOwner || isLegacy)) {
    throw new ForbiddenError()
  }
}

/**
 * Get the current user ID for setting createdBy on new records.
 * Returns undefined if no user is authenticated (dev mode).
 *
 * @returns Current user ID or undefined
 *
 * @example
 * ```typescript
 * const newTask = {
 *   ...taskData,
 *   createdBy: getCurrentUserIdForCreation(),
 *   createdAt: new Date(),
 * }
 * ```
 */
export function getCurrentUserIdForCreation(): string | undefined {
  return SessionContext.getCurrentUserId()
}
