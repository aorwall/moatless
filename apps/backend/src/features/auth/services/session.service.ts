import { randomBytes } from 'node:crypto'
import type { SessionResponse, User, UserSession } from '@moatless/types'
import { inject, singleton } from 'tsyringe'
import type { StorageBackend } from '../../../storage/index.js'
import { logger } from '../../../utils/logger.js'
import { SessionStorage } from '../storage/session.storage.js'
import type { OAuthTokens, ProviderUserInfo } from '../types/oauth.types.js'

@singleton()
export class SessionService {
  private readonly sessionMaxAge: number

  constructor(
    @inject(SessionStorage) private sessionStorage: SessionStorage,
    @inject('Storage') private userStorage: StorageBackend,
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
      const existingUser = await this.userStorage.getUser(userId)

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

      await this.userStorage.saveUser(userData)

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

      await this.sessionStorage.saveSession(session)

      const logMessage = groupId
        ? `Created session for user ${userInfo.login} (provider: ${providerId}, group: ${groupId})`
        : `Created session for user ${userInfo.login} (provider: ${providerId})`
      logger.info(logMessage)

      return sessionId
    } catch (error) {
      logger.error(`Error creating session for provider ${providerId}`, error as Error)
      return null
    }
  }

  /**
   * Get a session by ID
   */
  async getSession(sessionId: string): Promise<UserSession | null> {
    try {
      const session = await this.sessionStorage.getSession(sessionId)

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
      logger.error('Error getting session', error as Error)
      return null
    }
  }

  /**
   * Delete a session
   */
  async deleteSession(sessionId: string): Promise<void> {
    try {
      await this.sessionStorage.deleteSession(sessionId)
      logger.info(`Deleted session ${sessionId}`)
    } catch (error) {
      logger.error('Error deleting session', error as Error)
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
      const sessionKeys = await this.sessionStorage.listSessions()
      const now = new Date()
      let cleanedCount = 0

      for (const key of sessionKeys) {
        // Extract session ID from full key (sessions/xyz -> xyz)
        const sessionId = key.replace(/^sessions\//, '')
        const session = await this.sessionStorage.getSession(sessionId)

        if (session && new Date(session.expiresAt) < now) {
          await this.sessionStorage.deleteSession(sessionId)
          cleanedCount++
        }
      }

      if (cleanedCount > 0) {
        logger.info(`Cleaned up ${cleanedCount} expired sessions`)
      }
    } catch (error) {
      logger.error('Error cleaning up expired sessions', error as Error)
    }
  }
}
