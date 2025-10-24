import path from 'node:path'
import { singleton } from 'tsyringe'
import { SessionContext } from '../features/auth/index.js'
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
@singleton()
export class AuthorizedFileStorage extends FileStorage {
  /**
   * Override getPath to include userId prefix when a session exists
   * Throws error if no session exists (auth is required when using AuthorizedFileStorage)
   *
   * EXCEPTION: Session data (sessions/*) is stored at root level
   * to avoid circular dependency when loading sessions
   */
  protected override getPath(key: string, extension = '.json'): string {
    // Exception: sessions are stored at root level (not user-scoped)
    // This prevents circular dependency when auth middleware loads sessions
    if (key.startsWith('sessions/')) {
      return path.join(this.basePath, `${key}${extension}`)
    }

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

  /**
   * Override list to include userId prefix from session
   * Throws error if no session exists (same as getPath)
   */
  override async list(prefix?: string): Promise<string[]> {
    // Exception: sessions are stored at root level (not user-scoped)
    if (prefix?.startsWith('sessions/')) {
      return super.list(prefix)
    }

    const userId = SessionContext.getCurrentUserId()

    // Enforce that user session exists
    if (userId === undefined) {
      throw new Error(
        'Unauthorized: No user session found. All storage operations require authentication.',
      )
    }

    // Scope the prefix to the user
    const userScopedPrefix = prefix ? `users/${userId}/${prefix}` : `users/${userId}/`

    // Get files from the scoped directory
    const scopedKeys = await super.list(userScopedPrefix)

    // Strip the user scope prefix from returned keys
    const scopePrefix = `users/${userId}/`
    return scopedKeys.map((key) => key.replace(scopePrefix, ''))
  }
}
