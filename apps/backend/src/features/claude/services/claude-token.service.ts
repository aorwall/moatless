import { createHash, randomBytes } from 'node:crypto'
import type { ClaudeConnectionConfig, ClaudeOAuthUrlResponse } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { OAuthStateRepository } from '../../auth/db/oauth-state.repository'
import { ProviderTokenRepository } from '../../auth/db/provider-token.repository'
import { SessionContext } from '../../auth/services/session-context'

/**
 * Claude Token Service
 *
 * Manages Claude OAuth tokens and Personal Access Tokens for Claude Code integration.
 * Provides unified token resolution for Claude Code CLI with the following priority:
 *
 * Token Resolution Priority:
 * 1. Provider token from database (OAuth or PAT stored in provider_tokens table)
 * 2. Legacy token from settings.json file (for backward compatibility)
 * 3. null
 *
 * Features:
 * - OAuth flow with PKCE (Proof Key for Code Exchange)
 * - Personal Access Token (PAT) storage
 * - Automatic token refresh for OAuth tokens
 * - Connection status management
 */
@injectable()
export class ClaudeTokenService {
  // Claude OAuth constants (public client configuration from claude-code-login)
  private readonly CLIENT_ID = '9d1c250a-e61b-44d9-88ed-5944d1962f5e'
  private readonly REDIRECT_URI = 'https://console.anthropic.com/oauth/code/callback'
  private readonly SCOPES = 'org:create_api_key user:profile user:inference'
  private readonly AUTHORIZATION_URL = 'https://claude.ai/oauth/authorize'
  private readonly TOKEN_URL = 'https://console.anthropic.com/v1/oauth/token'

  constructor(
    @inject(ProviderTokenRepository) private readonly tokenRepository: ProviderTokenRepository,
    @inject(OAuthStateRepository) private readonly stateRepository: OAuthStateRepository,
  ) {}

  /**
   * Get Claude token for agent execution from database only.
   *
   * Returns OAuth or PAT token from provider_tokens table.
   * Does NOT include settings.json fallback - that logic stays in the agent.
   *
   * Priority:
   * 1. Provider token from database (OAuth or PAT with auto-refresh)
   * 2. null (agent will fall back to settings.json)
   *
   * @returns Token string or null if not available in database
   */
  async getTokenForAgent(): Promise<string | null> {
    const userId = SessionContext.getCurrentUserId()
    if (!userId) {
      logger.debug('No authenticated user for Claude token resolution')
      return null
    }

    // Check for provider token (OAuth or PAT)
    const providerToken = await this.tokenRepository.getProviderTokenRaw(userId, 'claude')
    if (providerToken) {
      // Check if OAuth token needs refresh
      if (
        providerToken.authMethod === 'oauth' &&
        providerToken.expiresAt &&
        providerToken.expiresAt <= new Date()
      ) {
        logger.info({ userId }, 'Claude OAuth token expired, attempting refresh')
        try {
          await this.refreshToken(userId)
          // Fetch the refreshed token
          const refreshedToken = await this.tokenRepository.getProviderTokenRaw(userId, 'claude')
          if (refreshedToken) {
            return refreshedToken.accessToken
          }
        } catch (error) {
          logger.error({ err: error as Error, userId }, 'Failed to refresh Claude token')
          return null
        }
      }

      logger.debug(
        {
          userId,
          method: providerToken.authMethod,
        },
        'Using Claude provider token',
      )
      return providerToken.accessToken
    }

    logger.debug({ userId }, 'No Claude token in database')
    return null
  }

  /**
   * Check if OAuth is enabled/configured on the server
   * @returns true (OAuth is always available for manual code entry)
   */
  isOAuthEnabled(): boolean {
    return true
  }

  /**
   * Get Claude connection configuration for current user.
   *
   * Returns connection status, method, and metadata about the Claude connection.
   * Used by the settings UI to display current connection state.
   *
   * @returns Configuration with connection status and method
   */
  async getConnectionConfig(): Promise<ClaudeConnectionConfig> {
    const userId = SessionContext.getCurrentUserId()
    if (!userId) {
      return {
        connected: false,
        oauthEnabled: this.isOAuthEnabled(),
      }
    }

    // Check for provider token
    const providerToken = await this.tokenRepository.getProviderTokenRaw(userId, 'claude')
    if (providerToken) {
      return {
        connected: true,
        method: providerToken.authMethod === 'oauth' ? 'oauth' : 'pat',
        claudeUserId: providerToken.providerUserId,
        connectedAt: providerToken.connectedAt.toISOString(),
        oauthEnabled: this.isOAuthEnabled(),
      }
    }

    return {
      connected: false,
      oauthEnabled: this.isOAuthEnabled(),
    }
  }

  /**
   * Generate Claude OAuth URL with PKCE for service connection.
   *
   * Creates OAuth flow for linking Claude to authenticated users.
   * Uses PKCE (Proof Key for Code Exchange) for enhanced security.
   *
   * @param userId - User ID initiating OAuth
   * @returns OAuth URL, state, and code verifier for PKCE
   */
  async generateServiceOAuthUrl(userId: string): Promise<ClaudeOAuthUrlResponse> {
    // Generate random state for CSRF protection (32 bytes = 64 hex chars)
    const state = randomBytes(32).toString('hex')

    // Generate PKCE code verifier (32 bytes base64url)
    const codeVerifier = randomBytes(32).toString('base64url')

    // Create SHA-256 hash of verifier as challenge
    const codeChallenge = createHash('sha256').update(codeVerifier).digest('base64url')

    // Store state and code verifier in oauth_states table (expires in 10 minutes)
    // We'll store the code verifier as part of the state string with a separator
    const expiresAt = new Date(Date.now() + 600000) // 10 minutes from now
    const stateWithVerifier = `${state}:${codeVerifier}`
    await this.stateRepository.saveState(stateWithVerifier, userId, 'claude', expiresAt)

    // Build OAuth URL with PKCE (redirect_uri shows code in address bar for manual copy)
    const params = new URLSearchParams({
      code: 'true',
      client_id: this.CLIENT_ID,
      response_type: 'code',
      redirect_uri: this.REDIRECT_URI,
      scope: this.SCOPES,
      code_challenge: codeChallenge,
      code_challenge_method: 'S256',
      state: stateWithVerifier,
    })

    const authUrl = `${this.AUTHORIZATION_URL}?${params.toString()}`

    logger.info({ userId, state: stateWithVerifier }, 'Generated Claude service OAuth URL')

    return { authUrl, state: stateWithVerifier }
  }

  /**
   * Save a Personal Access Token (PAT) for Claude.
   *
   * Allows users to manually enter a Claude token instead of using OAuth.
   *
   * @param userId - User ID
   * @param accessToken - Claude Personal Access Token
   */
  async savePersonalAccessToken(userId: string, accessToken: string): Promise<void> {
    await this.tokenRepository.saveProviderToken(userId, 'claude', {
      authMethod: 'pat',
      accessToken,
      tokenType: 'Bearer',
      connectedAt: new Date(),
      // PATs don't expire (until revoked) - omit expiresAt field
    })

    logger.info({ userId }, 'Claude Personal Access Token saved successfully')
  }

  /**
   * Disconnect Claude account by deleting the provider token.
   *
   * @param userId - User ID
   */
  async disconnect(userId: string): Promise<void> {
    await this.tokenRepository.deleteProviderToken(userId, 'claude')
    logger.info({ userId }, 'Claude account disconnected')
  }

  /**
   * Refresh an expired OAuth token using the refresh token.
   *
   * @param userId - User ID
   * @throws Error if refresh token is not available or refresh fails
   */
  async refreshToken(userId: string): Promise<void> {
    const providerToken = await this.tokenRepository.getProviderTokenRaw(userId, 'claude')
    if (!providerToken) {
      throw new Error('No Claude token found for user')
    }

    if (providerToken.authMethod !== 'oauth') {
      throw new Error('Cannot refresh Personal Access Token')
    }

    if (!providerToken.refreshToken) {
      throw new Error('No refresh token available')
    }

    try {
      const response = await fetch(this.TOKEN_URL, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json',
        },
        body: JSON.stringify({
          grant_type: 'refresh_token',
          client_id: this.CLIENT_ID,
          refresh_token: providerToken.refreshToken,
        }),
      })

      if (!response.ok) {
        const errorText = await response.text()
        logger.error(
          {
            err: new Error(`Status: ${response.status}`),
            status: response.status,
            error: errorText,
          },
          'Claude token refresh failed',
        )
        throw new Error('Failed to refresh Claude token')
      }

      const tokenResponse = (await response.json()) as {
        access_token: string
        refresh_token: string
        expires_in: number
        scope?: string
        token_type?: string
      }

      // Calculate new expiration time
      const expiresAt = new Date(Date.now() + tokenResponse.expires_in * 1000)

      // Update provider token with new tokens
      await this.tokenRepository.saveProviderToken(userId, 'claude', {
        authMethod: 'oauth',
        accessToken: tokenResponse.access_token,
        refreshToken: tokenResponse.refresh_token,
        tokenType: tokenResponse.token_type || 'Bearer',
        scope: tokenResponse.scope || this.SCOPES,
        connectedAt: providerToken.connectedAt, // Keep original connection time
        expiresAt,
        ...(providerToken.providerUserId && { providerUserId: providerToken.providerUserId }),
        ...(providerToken.providerLogin && { providerLogin: providerToken.providerLogin }),
      })

      logger.info(
        {
          userId,
          expiresAt: expiresAt.toISOString(),
        },
        'Claude token refreshed successfully',
      )
    } catch (error) {
      logger.error({ err: error as Error, userId }, 'Error refreshing Claude token')
      throw error
    }
  }
}
