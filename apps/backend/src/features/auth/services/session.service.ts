import { randomBytes } from 'node:crypto'
import type { SessionResponse, User, UserSession } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { SessionRepository } from '../db/session.repository'
import { UserRepository } from '../db/user.repository'
import type { OAuthTokens, ProviderUserInfo } from '../types/oauth.types'

@injectable()
export class SessionService {
  private readonly sessionMaxAge: number

  constructor(
    @inject(SessionRepository) private sessionRepository: SessionRepository,
    @inject(UserRepository) private userRepository: UserRepository,
  ) {
    // SESSION_MAX_AGE is in seconds, convert to milliseconds for internal use
    // Default to 7 days (604800 seconds) if not configured
    const maxAgeSeconds = Number.parseInt(process.env['SESSION_MAX_AGE'] || '', 10) || 604800
    this.sessionMaxAge = maxAgeSeconds * 1000
  }

  /**
   * Generate a cryptographically secure session ID
   */
  private generateSessionId(): string {
    return randomBytes(32).toString('hex')
  }

  /**
   * Create a new user session for any OAuth provider
   * @param providerId - Provider identifier (e.g., "github", "auth0")
   * @param tokens - OAuth tokens from the provider
   * @param userInfo - Normalized user information from the provider
   * @param groupId - Optional group ID for group-based access control
   */
  async createSession(
    providerId: string,
    tokens: OAuthTokens,
    userInfo: ProviderUserInfo,
    groupId?: string,
  ): Promise<string | null> {
    try {
      // Save or update user data
      const now = new Date()
      const userId = userInfo.id
      const existingUser = await this.userRepository.getUser(userId)

      const userData: User = existingUser
        ? {
            ...existingUser,
            // Update mutable fields
            login: userInfo.login,
            email: userInfo.email,
            name: userInfo.name,
            avatarUrl: userInfo.avatarUrl,
            lastLoginAt: now,
          }
        : {
            // Create new user
            id: userId,
            login: userInfo.login,
            email: userInfo.email,
            name: userInfo.name,
            avatarUrl: userInfo.avatarUrl,
            provider: providerId,
            createdAt: now,
            lastLoginAt: now,
          }

      await this.userRepository.saveUser(userData)

      const logAction = existingUser ? 'Updated' : 'Created'
      logger.info(
        `${logAction} user record for ${userInfo.login} (id: ${userId}, provider: ${providerId})`,
      )

      const sessionId = this.generateSessionId()
      const expiresAt = new Date(now.getTime() + this.sessionMaxAge)

      const session: UserSession = {
        id: sessionId,
        provider: providerId,
        userId,
        userLogin: userInfo.login,
        userEmail: userInfo.email,
        userName: userInfo.name,
        avatarUrl: userInfo.avatarUrl,
        accessToken: tokens.accessToken,
        idToken: tokens.idToken,
        tokenType: tokens.tokenType,
        scope: tokens.scope,
        ...(groupId !== undefined && { groupId }), // Only add groupId if defined
        createdAt: now,
        expiresAt,
      }

      await this.sessionRepository.saveSession(session)

      const logMessage = groupId
        ? `Created session for user ${userInfo.login} (provider: ${providerId}, group: ${groupId})`
        : `Created session for user ${userInfo.login} (provider: ${providerId})`
      logger.info(logMessage)

      return sessionId
    } catch (error) {
      logger.error(
        { err: error as Error, providerId },
        `Error creating session for provider ${providerId}`,
      )
      return null
    }
  }

  /**
   * Get a session by ID
   */
  async getSession(sessionId: string): Promise<UserSession | null> {
    try {
      const session = await this.sessionRepository.getSession(sessionId)

      if (!session) {
        return null
      }

      // Check if session has expired
      const now = new Date()
      const expiresAt = new Date(session.expiresAt)

      if (now > expiresAt) {
        logger.info(`Session ${sessionId} expired`)
        await this.deleteSession(sessionId)
        return null
      }

      return session
    } catch (error) {
      logger.error({ err: error as Error }, 'Error getting session')
      return null
    }
  }

  /**
   * Delete a session
   */
  async deleteSession(sessionId: string): Promise<void> {
    try {
      await this.sessionRepository.deleteSession(sessionId)
      logger.info(`Deleted session ${sessionId}`)
    } catch (error) {
      logger.error({ err: error as Error }, 'Error deleting session')
    }
  }

  /**
   * Convert session to API response format
   */
  sessionToResponse(session: UserSession): SessionResponse {
    return {
      user: {
        id: session.userId,
        login: session.userLogin,
        email: session.userEmail,
        name: session.userName,
        avatarUrl: session.avatarUrl,
        provider: session.provider,
      },
      expiresAt: session.expiresAt.toISOString(),
    }
  }

  /**
   * Cleanup expired sessions (should be called periodically)
   */
  async cleanupExpiredSessions(): Promise<void> {
    try {
      const cleanedCount = await this.sessionRepository.deleteExpiredSessions()

      if (cleanedCount > 0) {
        logger.info(`Cleaned up ${cleanedCount} expired sessions`)
      }
    } catch (error) {
      logger.error({ err: error as Error }, 'Error cleaning up expired sessions')
    }
  }
}
