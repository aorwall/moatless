import path from 'node:path'
import { SessionContext } from '../services/access-control/index.js'
import { FileStorage } from './file-storage.js'

/**
 * User-scoped file storage that isolates data by userId
 * Extends FileStorage to automatically prefix paths with users/{userId}/
 *
 * SECURITY: Throws error if no user session exists - all operations require authentication
 *
 * Path example with session (userId: 456):
 * tasks/123.json → data/users/456/tasks/123.json
 */
export class AuthorizedFileStorage extends FileStorage {
  /**
   * Override getPath to include userId prefix when a session exists
   * Throws error if no session exists (auth is required when using AuthorizedFileStorage)
   */
  protected override getPath(key: string, extension = '.json'): string {
    const userId = SessionContext.getCurrentUserId()

    // Enforce that user session exists - if AuthorizedFileStorage is used,
    // all operations must be associated with an authenticated user
    if (userId === undefined) {
      throw new Error(
        'Unauthorized: No user session found. All storage operations require authentication.',
      )
    }

    // Scope the path to the user
    const userScopedKey = `users/${userId}/${key}`
    return path.join(this.basePath, `${userScopedKey}${extension}`)
  }
}

/**
 * Factory function to create AuthorizedFileStorage
 */
export async function createAuthorizedStorageService(
  basePath: string,
): Promise<AuthorizedFileStorage> {
  const { promises: fs } = await import('node:fs')

  // Ensure the data directory exists
  await fs.mkdir(basePath, { recursive: true })

  return new AuthorizedFileStorage(basePath)
}
