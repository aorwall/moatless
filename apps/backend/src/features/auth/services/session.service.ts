import { randomBytes } from 'node:crypto'
import type { SessionResponse, User, UserSession } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { ProviderTokenRepository } from '../db/provider-token.repository'
import { SessionRepository } from '../db/session.repository'
import { UserRepository } from '../db/user.repository'
import type { OAuthTokens, ProviderUserInfo } from '../types/oauth.types'

@injectable()
export class SessionService {
  private readonly sessionMaxAge: number

  constructor(
    @inject(SessionRepository) private sessionRepository: SessionRepository,
    @inject(UserRepository) private userRepository: UserRepository,
    @inject('ProviderTokenRepository') private providerTokenRepository: ProviderTokenRepository,
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
            role: 'user', // New users get 'user' role by default
            createdAt: now,
            lastLoginAt: now,
          }

      // Get role from user data (existing or newly created)
      const userRole = userData.role

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
        role: userRole, // Cache role in session for fast access
        accessToken: tokens.accessToken,
        idToken: tokens.idToken,
        tokenType: tokens.tokenType,
        scope: tokens.scope,
        ...(groupId !== undefined && { groupId }), // Only add groupId if defined
        createdAt: now,
        expiresAt,
      }

      await this.sessionRepository.saveSession(session)

      // For GitHub OAuth logins, also persist token to provider_tokens table
      // This makes the token available for non-browser contexts (Slack, API keys, etc.)
      if (providerId === 'github' && tokens.accessToken) {
        try {
          await this.providerTokenRepository.saveProviderToken(userId, 'github', {
            authMethod: 'oauth',
            accessToken: tokens.accessToken,
            tokenType: tokens.tokenType,
            ...(tokens.scope && { scope: tokens.scope }),
            connectedAt: now,
            providerUserId: userId,
            providerLogin: userInfo.login,
          })
          logger.info(
            { userId, login: userInfo.login },
            'Persisted GitHub OAuth token to provider_tokens for cross-context access',
          )
        } catch (tokenError) {
          // Log but don't fail session creation - browser session still works
          logger.warn(
            { err: tokenError, userId },
            'Failed to persist GitHub OAuth token to provider_tokens',
          )
        }
      }

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
        role: session.role,
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
