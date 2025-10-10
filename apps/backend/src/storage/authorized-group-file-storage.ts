import path from 'node:path'
import { SessionContext } from '../services/access-control/index.js'
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
export class AuthorizedGroupFileStorage extends FileStorage {
  /**
   * Override getPath to include groupId prefix from session
   * Throws error if no session exists or if user has no group
   */
  protected override getPath(key: string, extension = '.json'): string {
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
}

/**
 * Factory function to create AuthorizedGroupFileStorage
 */
export async function createAuthorizedGroupStorageService(
  basePath: string,
): Promise<AuthorizedGroupFileStorage> {
  const { promises: fs } = await import('node:fs')

  // Ensure the data directory exists
  await fs.mkdir(basePath, { recursive: true })

  return new AuthorizedGroupFileStorage(basePath)
}
