import type { User } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { UserRepository } from '../../auth/db/user.repository'

/**
 * Service for resolving external user identifiers to system users.
 * Used by integrations to map email addresses and Slack users to system users.
 */
@injectable()
export class UserResolverService {
  constructor(@inject(UserRepository) private userRepository: UserRepository) {}

  /**
   * Resolve a user by their login name.
   * Used for email integration: {username}.{repository}@inbox.moatless.ai
   *
   * @param login - The login/username to look up
   * @returns The user if found, null otherwise
   */
  async resolveByLogin(login: string): Promise<User | null> {
    try {
      const users = await this.userRepository.findByLogin(login)
      const user = users[0]
      if (user) {
        logger.debug({ login }, 'User resolved by login')
        return user
      }
      logger.debug({ login }, 'User not found by login')
      return null
    } catch (error) {
      logger.error({ err: error, login }, 'Error resolving user by login')
      return null
    }
  }

  /**
   * Resolve a user by their email address.
   * Used for Slack integration where we match Slack user email to system user.
   *
   * @param email - The email address to look up
   * @returns The user if found, null otherwise
   */
  async resolveByEmail(email: string): Promise<User | null> {
    try {
      const user = await this.userRepository.findByEmail(email)
      if (user) {
        logger.debug({ email }, 'User resolved by email')
        return user
      }
      logger.debug({ email }, 'User not found by email')
      return null
    } catch (error) {
      logger.error({ err: error, email }, 'Error resolving user by email')
      return null
    }
  }
}
