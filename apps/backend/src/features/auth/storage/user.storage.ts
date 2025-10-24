import type { User } from '@moatless/types'
import { inject, singleton } from 'tsyringe'
import type { Storage } from '../../../infrastructure/storage/storage.js'

/**
 * Feature-scoped storage adapter for user persistence.
 * Encapsulates key structure and provides type-safe user operations.
 */
@singleton()
export class UserStorage {
  private readonly usersPrefix = 'users/'

  constructor(@inject('Storage') private readonly storage: Storage) {}

  /**
   * Save a user to storage
   */
  async saveUser(user: User): Promise<void> {
    await this.storage.save(this.key(user.id), user)
  }

  /**
   * Get a user by ID
   */
  async getUser(userId: string): Promise<User | null> {
    return await this.storage.get<User>(this.key(userId))
  }

  /**
   * Generate storage key for a user ID
   */
  private key(userId: string): string {
    return `${this.usersPrefix}${userId}`
  }
}
