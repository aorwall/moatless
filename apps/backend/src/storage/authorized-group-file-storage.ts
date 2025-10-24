import path from 'node:path'
import { singleton } from 'tsyringe'
import { SessionContext } from '../features/auth/index.js'
import { FileStorage } from './file-storage.js'

/**
 * Group-scoped file storage that isolates data by groupId
 * Extends FileStorage to automatically prefix paths with groups/{groupId}/
 *
 * SECURITY: Throws error if no user session exists OR if user has no group assigned
 * All operations require authentication AND group membership
 *
 * Path example with group (groupId: "team-frontend"):
 * tasks/123.json → data/groups/team-frontend/tasks/123.json
 */
@singleton()
export class AuthorizedGroupFileStorage extends FileStorage {
  /**
   * Override getPath to include groupId prefix from session
   * Throws error if no session exists or if user has no group
   *
   * EXCEPTION: Session data (sessions/*) and user data (users/*) are stored at root level
   * to avoid circular dependency when loading sessions
   */
  protected override getPath(key: string, extension = '.json'): string {
    // Exception: sessions and users are stored at root level (not group-scoped)
    // This prevents circular dependency when auth middleware loads sessions
    if (key.startsWith('sessions/') || key.startsWith('users/')) {
      return path.join(this.basePath, `${key}${extension}`)
    }

    const groupId = SessionContext.getCurrentGroupId()

    // Enforce that group exists - if AuthorizedGroupFileStorage is used,
    // all operations must be associated with a group
    if (groupId === undefined) {
      const userId = SessionContext.getCurrentUserId()
      if (userId === undefined) {
        throw new Error(
          'Unauthorized: No user session found. All storage operations require authentication.',
        )
      }

      throw new Error(
        `Unauthorized: User ${userId} is not assigned to any group. Group membership is required for access.`,
      )
    }

    // Scope the path to the group
    const groupScopedKey = `groups/${groupId}/${key}`
    return path.join(this.basePath, `${groupScopedKey}${extension}`)
  }

  /**
   * Override list to include groupId prefix from session
   * Throws error if no session exists or user has no group (same as getPath)
   */
  override async list(prefix?: string): Promise<string[]> {
    // Exception: sessions and users are stored at root level (not group-scoped)
    if (prefix?.startsWith('sessions/') || prefix?.startsWith('users/')) {
      return super.list(prefix)
    }

    const groupId = SessionContext.getCurrentGroupId()

    // Enforce that group exists
    if (groupId === undefined) {
      const userId = SessionContext.getCurrentUserId()
      if (userId === undefined) {
        throw new Error(
          'Unauthorized: No user session found. All storage operations require authentication.',
        )
      }

      throw new Error(
        `Unauthorized: User ${userId} is not assigned to any group. Group membership is required for access.`,
      )
    }

    // Scope the prefix to the group
    const groupScopedPrefix = prefix ? `groups/${groupId}/${prefix}` : `groups/${groupId}/`

    // Get files from the scoped directory
    const scopedKeys = await super.list(groupScopedPrefix)

    // Strip the group scope prefix from returned keys
    const scopePrefix = `groups/${groupId}/`
    return scopedKeys.map((key) => key.replace(scopePrefix, ''))
  }
}
