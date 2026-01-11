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

/**
 * Capture the current session for use in background/async operations.
 * Returns a wrapper function that re-establishes session context.
 *
 * Use this when you need to spawn async work that should maintain the
 * current user's session context for access control and ownership tracking.
 *
 * @returns A wrapper function that runs the provided function within the captured session context
 *
 * @example
 * ```typescript
 * const withSession = captureSessionForBackground()
 * void (async () => {
 *   await withSession(async () => {
 *     // Session context available here
 *     const userId = SessionContext.getCurrentUserId()
 *     await taskRepository.save(task) // Uses correct user context
 *   })
 * })()
 * ```
 */
export function captureSessionForBackground(): <T>(fn: () => T | Promise<T>) => Promise<T> {
  const session = SessionContext.getCurrentSession()
  return async <T>(fn: () => T | Promise<T>): Promise<T> => {
    if (session) {
      return await SessionContext.run(session, fn)
    }
    return await fn()
  }
}

/**
 * Run an async function in the background while preserving session context.
 * This is a convenience wrapper for fire-and-forget operations.
 *
 * Use this instead of `void (async () => {...})()` when the background
 * operation needs access to the current user's session for:
 * - Access control checks (e.g., taskRepository.get with visibility scope)
 * - Setting createdBy on new records
 * - Ownership assertions
 *
 * @param fn - Async function to run in background
 *
 * @example
 * ```typescript
 * // Instead of:
 * void (async () => {
 *   await this.taskRepository.save(task) // Session context LOST!
 * })()
 *
 * // Use:
 * runInBackgroundWithSession(async () => {
 *   await this.taskRepository.save(task) // Session context preserved
 * })
 * ```
 */
export function runInBackgroundWithSession(fn: () => Promise<void>): void {
  const session = SessionContext.getCurrentSession()
  // biome-ignore lint/complexity/noVoid: Background task needs to run without awaiting
  void (async () => {
    if (session) {
      await SessionContext.run(session, fn)
    } else {
      await fn()
    }
  })()
}
