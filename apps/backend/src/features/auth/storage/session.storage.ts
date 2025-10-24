import type { UserSession } from '@moatless/types'
import { inject, singleton } from 'tsyringe'
import type { Storage } from '../../../infrastructure/storage/storage.js'

/**
 * Feature-scoped storage adapter for session persistence.
 * Encapsulates key structure and provides type-safe session operations.
 */
@singleton()
export class SessionStorage {
  private readonly sessionsPrefix = 'sessions/'

  constructor(@inject('Storage') private readonly storage: Storage) {}

  /**
   * Save a session to storage
   */
  async saveSession(session: UserSession): Promise<void> {
    await this.storage.save(this.key(session.id), session)
  }

  /**
   * Get a session by ID
   */
  async getSession(id: string): Promise<UserSession | null> {
    return await this.storage.get<UserSession>(this.key(id))
  }

  /**
   * Delete a session by ID
   */
  async deleteSession(id: string): Promise<void> {
    await this.storage.delete(this.key(id))
  }

  /**
   * Check if a session exists
   */
  async exists(id: string): Promise<boolean> {
    return await this.storage.exists(this.key(id))
  }

  /**
   * List all session keys matching the optional prefix
   * Used for cleanup operations
   */
  async listSessions(prefix?: string): Promise<string[]> {
    const searchPrefix = prefix ? `${this.sessionsPrefix}${prefix}` : this.sessionsPrefix
    return await this.storage.list(searchPrefix)
  }

  /**
   * Generate storage key for a session ID
   */
  private key(id: string): string {
    return `${this.sessionsPrefix}${id}`
  }
}
