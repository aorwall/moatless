import type { SessionResponse, UserSession } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { OAuthProvider } from '../providers/oauth-provider.interface'
import { OAuthProvidersSymbol } from '../providers/oauth-provider.interface'
import { SessionService } from './session.service'
import { SessionContext } from './session-context'

/**
 * Authentication service using provider abstraction
 * Works with any OAuth provider that implements the OAuthProvider interface
 */
@injectable()
export class AuthService {
  private sessionService: SessionService

  constructor(
    @inject(OAuthProvidersSymbol) private providers: Map<string, OAuthProvider>,
    @inject(SessionService) sessionService: SessionService,
  ) {
    this.sessionService = sessionService
  }

  /**
   * Get provider by ID, throwing error if not found
   */
  private getProvider(providerId: string): OAuthProvider {
    const provider = this.providers.get(providerId)
    if (!provider) {
      throw new Error(`Unknown auth provider: ${providerId}`)
    }
    return provider
  }

  /**
   * Generate CSRF state token for OAuth flow
   */
  generateOAuthState(): string {
    return require('node:crypto').randomBytes(16).toString('hex')
  }

  /**
   * Generate OAuth authorization URL for a specific provider
   */
  generateAuthUrl(providerId: string, state: string): string | null {
    const provider = this.getProvider(providerId)
    return provider.generateAuthUrl(state)
  }

  // Session methods - delegated to SessionService
  async getSession(sessionId: string): Promise<UserSession | null> {
    return await this.sessionService.getSession(sessionId)
  }

  async deleteSession(sessionId: string): Promise<void> {
    return await this.sessionService.deleteSession(sessionId)
  }

  sessionToResponse(session: UserSession): SessionResponse {
    return this.sessionService.sessionToResponse(session)
  }

  async cleanupExpiredSessions(): Promise<void> {
    return await this.sessionService.cleanupExpiredSessions()
  }

  /**
   * Handle OAuth callback for any provider
   * Validates state, exchanges code for tokens, creates session
   */
  async handleCallback(
    providerId: string,
    code: string,
    state: string | undefined,
    storedState: string | undefined,
  ): Promise<{ sessionId: string; redirectUrl: string; sessionMaxAge: number }> {
    // Verify state parameter for CSRF protection
    if (state && state !== storedState) {
      logger.warn('OAuth state mismatch - possible CSRF attack')
      throw new Error('Invalid state parameter')
    }

    const provider = this.getProvider(providerId)

    // Exchange code for tokens (provider-specific)
    const tokens = await provider.exchangeCodeForTokens(code)

    // Get user info (provider-specific)
    const userInfo = await provider.getUserInfo(tokens)

    // Extract group ID if provider supports it (optional)
    const groupId = provider.extractUserGroup ? await provider.extractUserGroup(tokens) : undefined

    // Create session (generic)
    const sessionId = await this.sessionService.createSession(providerId, tokens, userInfo, groupId)

    if (!sessionId) {
      throw new Error('Failed to create session')
    }

    // Get session max age for cookie
    const sessionMaxAge = Number.parseInt(process.env['SESSION_MAX_AGE'] || '', 10) || 604800

    // Return session info and redirect URL
    const frontendUrl = process.env['FRONTEND_URL'] || 'http://localhost:3000'
    return { sessionId, redirectUrl: frontendUrl, sessionMaxAge }
  }

  /**
   * Handle logout
   * Gets session provider and deletes session, returns logout URL if provider supports it
   */
  async handleLogout(
    sessionId: string | undefined,
  ): Promise<{ logoutUrl?: string | undefined; provider?: string | undefined }> {
    // In dev mode (auth disabled), no session to delete, just return success
    const authProvider = process.env['AUTH_PROVIDER']
    if (!authProvider || authProvider === 'disabled') {
      return { logoutUrl: undefined, provider: 'dev' }
    }

    // Get session to determine provider before deleting
    let providerId: string | undefined
    if (sessionId) {
      const session = await this.getSession(sessionId)
      if (session) {
        providerId = session.provider
      }
      // Delete session in all cases
      await this.deleteSession(sessionId)
    }

    // If provider supports logout URL, generate it
    if (providerId) {
      const provider = this.providers.get(providerId)
      if (provider?.generateLogoutUrl) {
        const frontendUrl = process.env['FRONTEND_URL'] || 'http://localhost:3000'
        const logoutUrl = provider.generateLogoutUrl(frontendUrl)

        if (logoutUrl) {
          logger.info(`Redirecting to ${providerId} logout`)
          return { logoutUrl, provider: providerId }
        }

        logger.warn(`Failed to generate ${providerId} logout URL, falling back to JSON response`)
      }
    }

    return { logoutUrl: undefined, provider: providerId }
  }

  /**
   * Get current authenticated user
   * Handles dev mode (no auth) and validates session
   */
  async getCurrentUserSession(sessionId: string | undefined): Promise<SessionResponse> {
    // In development mode (when no auth is enabled or disabled), check for dev session
    const authProvider = process.env['AUTH_PROVIDER']

    if (!authProvider || authProvider === 'disabled') {
      // Try to get dev session from context (set by dev-session middleware)
      const devSession = SessionContext.getCurrentSession()

      if (devSession) {
        return this.sessionToResponse(devSession)
      }

      // No dev session available, return null user
      return {
        user: null,
        expiresAt: null,
      }
    }

    // Production mode - require valid session
    if (!sessionId) {
      throw new Error('Not authenticated')
    }

    const session = await this.getSession(sessionId)

    if (!session) {
      throw new Error('Session expired or invalid')
    }

    return this.sessionToResponse(session)
  }
}
