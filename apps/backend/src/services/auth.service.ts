import { randomBytes } from 'node:crypto'
import type {
  GitHubOAuthTokenResponse,
  GitHubOAuthUser,
  SessionResponse,
  User,
  UserSession,
} from '@moatless/types'
import type { StorageBackend } from '../storage/index.js'
import { logger } from '../utils/logger.js'
import type { GitHubService } from './github.service.js'

export class AuthService {
  private readonly sessionsPrefix = 'sessions/'
  private readonly sessionMaxAge: number

  constructor(
    private storage: StorageBackend,
    private github: GitHubService,
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
   * Generate a random state parameter for OAuth CSRF protection
   */
  generateOAuthState(): string {
    return randomBytes(16).toString('hex')
  }

  /**
   * Generate GitHub OAuth authorization URL
   */
  generateGitHubAuthUrl(state: string): string | null {
    const clientId = process.env['GITHUB_CLIENT_ID']
    const callbackUrl = process.env['GITHUB_CALLBACK_URL']

    if (!(clientId && callbackUrl)) {
      logger.error('GitHub OAuth credentials not configured')
      return null
    }

    const params = new URLSearchParams({
      client_id: clientId,
      redirect_uri: callbackUrl,
      state,
      scope: 'user:email',
    })

    return `https://github.com/login/oauth/authorize?${params.toString()}`
  }

  /**
   * Exchange GitHub OAuth code for access token
   */
  async exchangeCodeForToken(code: string): Promise<GitHubOAuthTokenResponse | null> {
    const clientId = process.env['GITHUB_CLIENT_ID']
    const clientSecret = process.env['GITHUB_CLIENT_SECRET']

    if (!(clientId && clientSecret)) {
      logger.error('GitHub OAuth credentials not configured')
      return null
    }

    try {
      const response = await fetch('https://github.com/login/oauth/access_token', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json',
        },
        body: JSON.stringify({
          client_id: clientId,
          client_secret: clientSecret,
          code,
        }),
      })

      if (!response.ok) {
        logger.error('Failed to exchange code for token', new Error(`Status: ${response.status}`))
        return null
      }

      const data = (await response.json()) as GitHubOAuthTokenResponse & {
        error?: string
      }

      if (data.error) {
        logger.error('GitHub OAuth error', new Error(data.error))
        return null
      }

      return {
        access_token: data.access_token,
        token_type: data.token_type,
        scope: data.scope,
      }
    } catch (error) {
      logger.error('Error exchanging code for token', error as Error)
      return null
    }
  }

  /**
   * Create a new user session
   * @param groupId Optional group ID for group-based access control
   */
  async createSession(
    accessToken: string,
    tokenType: string,
    scope: string,
    groupId?: string,
  ): Promise<string | null> {
    try {
      // Fetch user information from GitHub
      const user = await this.github.getAuthenticatedUser(accessToken)

      if (!user) {
        logger.error('Failed to fetch user information')
        return null
      }

      // Cast to GitHubOAuthUser (getAuthenticatedUser returns compatible data)
      const oauthUser = user as unknown as GitHubOAuthUser

      // Save or update user data
      const now = new Date()
      const existingUser = await this.storage.getUser(oauthUser.id)

      const userData: User = existingUser
        ? {
            ...existingUser,
            // Update mutable fields
            login: oauthUser.login,
            email: oauthUser.email,
            name: oauthUser.name,
            avatarUrl: oauthUser.avatar_url,
            lastLoginAt: now,
          }
        : {
            // Create new user
            id: oauthUser.id,
            login: oauthUser.login,
            email: oauthUser.email,
            name: oauthUser.name,
            avatarUrl: oauthUser.avatar_url,
            createdAt: now,
            lastLoginAt: now,
          }

      await this.storage.saveUser(userData)

      const logAction = existingUser ? 'Updated' : 'Created'
      logger.info(`${logAction} user record for ${oauthUser.login} (id: ${oauthUser.id})`)

      const sessionId = this.generateSessionId()
      const expiresAt = new Date(now.getTime() + this.sessionMaxAge)

      const session: UserSession = {
        id: sessionId,
        userId: oauthUser.id,
        userLogin: oauthUser.login,
        userEmail: oauthUser.email,
        userName: oauthUser.name,
        avatarUrl: oauthUser.avatar_url,
        accessToken,
        tokenType,
        scope,
        ...(groupId !== undefined && { groupId }), // Only add groupId if defined
        createdAt: now,
        expiresAt,
        lastAccessedAt: now,
      }

      await this.storage.save(`${this.sessionsPrefix}${sessionId}`, session)

      const logMessage = groupId
        ? `Created session for user ${oauthUser.login} (group: ${groupId})`
        : `Created session for user ${oauthUser.login}`
      logger.info(logMessage)

      return sessionId
    } catch (error) {
      logger.error('Error creating session', error as Error)
      return null
    }
  }

  /**
   * Get a session by ID
   */
  async getSession(sessionId: string): Promise<UserSession | null> {
    try {
      const session = await this.storage.get<UserSession>(`${this.sessionsPrefix}${sessionId}`)

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

      // Update last accessed time
      session.lastAccessedAt = now
      await this.storage.save(`${this.sessionsPrefix}${sessionId}`, session)

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
      await this.storage.delete(`${this.sessionsPrefix}${sessionId}`)
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
      },
      expiresAt: session.expiresAt.toISOString(),
    }
  }

  /**
   * Cleanup expired sessions (should be called periodically)
   */
  async cleanupExpiredSessions(): Promise<void> {
    try {
      const sessionKeys = await this.storage.list(this.sessionsPrefix)
      const now = new Date()
      let cleanedCount = 0

      for (const key of sessionKeys) {
        const session = await this.storage.get<UserSession>(key)

        if (session && new Date(session.expiresAt) < now) {
          await this.storage.delete(key)
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

/**
 * Factory function to create AuthService
 */
export function createAuthService(storage: StorageBackend, github: GitHubService): AuthService {
  return new AuthService(storage, github)
}
