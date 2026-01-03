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
  private readonly API_KEY_URL = 'https://api.anthropic.com/api/oauth/claude_cli/create_api_key'

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
      // Map auth method to connection method
      let method: 'oauth' | 'pat' | 'api_key' | 'file'
      switch (providerToken.authMethod) {
        case 'oauth':
          method = 'oauth'
          break
        case 'api_key':
          method = 'api_key'
          break
        default:
          method = 'pat'
      }

      return {
        connected: true,
        method,
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
   * Exchange an authorization code for OAuth tokens.
   */
  async exchangeCodeForTokens(
    authorizationCode: string,
    codeVerifier: string,
    state: string,
  ): Promise<{ accessToken: string; refreshToken?: string; expiresIn?: number } | null> {
    try {
      const response = await fetch(this.TOKEN_URL, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json',
        },
        body: JSON.stringify({
          grant_type: 'authorization_code',
          code: authorizationCode,
          redirect_uri: this.REDIRECT_URI,
          client_id: this.CLIENT_ID,
          code_verifier: codeVerifier,
          state: state,
        }),
      })

      if (!response.ok) {
        const errorText = await response.text()
        logger.error(
          { status: response.status, error: errorText },
          'Failed to exchange authorization code for tokens',
        )
        return null
      }

      const data = (await response.json()) as {
        access_token: string
        refresh_token?: string
        expires_in?: number
      }

      logger.info('Successfully exchanged authorization code for OAuth tokens')

      return {
        accessToken: data.access_token,
        ...(data.refresh_token && { refreshToken: data.refresh_token }),
        ...(data.expires_in && { expiresIn: data.expires_in }),
      }
    } catch (error) {
      logger.error({ err: error as Error }, 'Error exchanging authorization code')
      return null
    }
  }

  /**
   * Create an API key from an OAuth token.
   */
  async createApiKeyFromOAuthToken(oauthToken: string): Promise<string | null> {
    try {
      const response = await fetch(this.API_KEY_URL, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${oauthToken}`,
          'Content-Type': 'application/json',
        },
      })

      if (!response.ok) {
        const errorText = await response.text()
        logger.error(
          { status: response.status, error: errorText },
          'Failed to create API key from OAuth token',
        )
        return null
      }

      const data = (await response.json()) as { raw_key: string }
      return data.raw_key
    } catch (error) {
      logger.error({ err: error as Error }, 'Error creating API key from OAuth token')
      return null
    }
  }

  /**
   * Parse an authorization code input and extract components.
   * Format: <code>#<state>:<verifier>
   */
  private parseAuthorizationCodeInput(input: string): {
    code: string
    state: string
    verifier: string
  } {
    const [authorizationCode, stateWithVerifier] = input.split('#')
    if (!(authorizationCode && stateWithVerifier)) {
      throw new Error('Invalid authorization code format')
    }

    const colonIndex = stateWithVerifier.indexOf(':')
    if (colonIndex === -1) {
      throw new Error('Invalid state format - missing code verifier')
    }

    return {
      code: authorizationCode,
      state: stateWithVerifier,
      verifier: stateWithVerifier.substring(colonIndex + 1),
    }
  }

  /**
   * Process an authorization code: exchange for tokens, then convert to API key.
   */
  private async processAuthorizationCode(
    userId: string,
    code: string,
    verifier: string,
    state: string,
  ): Promise<{
    token: string
    authMethod: 'oauth' | 'api_key'
    refreshToken?: string
    expiresAt?: Date
  }> {
    const tokens = await this.exchangeCodeForTokens(code, verifier, state)
    if (!tokens) {
      throw new Error('Failed to exchange authorization code for tokens')
    }

    logger.info({ userId }, 'OAuth tokens received, converting to API key')
    const apiKey = await this.createApiKeyFromOAuthToken(tokens.accessToken)

    if (apiKey) {
      logger.info({ userId }, 'Successfully converted OAuth token to API key')
      return { token: apiKey, authMethod: 'api_key' }
    }

    logger.warn({ userId }, 'Failed to create API key, storing OAuth token directly')
    return {
      token: tokens.accessToken,
      authMethod: 'oauth',
      ...(tokens.refreshToken && { refreshToken: tokens.refreshToken }),
      ...(tokens.expiresIn && { expiresAt: new Date(Date.now() + tokens.expiresIn * 1000) }),
    }
  }

  /**
   * Save a Claude token. Handles multiple input formats:
   * 1. Authorization code with state: <code>#<state>:<verifier>
   * 2. OAuth token (sk-ant-oat...) - converts to API key
   * 3. API key or PAT - stores directly
   */
  async savePersonalAccessToken(userId: string, input: string): Promise<void> {
    let tokenToStore = input
    let authMethod: 'pat' | 'oauth' | 'api_key' = 'pat'
    let refreshToken: string | undefined
    let expiresAt: Date | undefined

    // Check if input is an authorization code with state
    if (input.includes('#')) {
      logger.info({ userId }, 'Authorization code detected, exchanging for tokens')
      const { code, verifier, state } = this.parseAuthorizationCodeInput(input)
      const result = await this.processAuthorizationCode(userId, code, verifier, state)
      tokenToStore = result.token
      authMethod = result.authMethod
      refreshToken = result.refreshToken
      expiresAt = result.expiresAt
    }
    // Check if input is an OAuth token (sk-ant-oat...)
    else if (input.startsWith('sk-ant-oat')) {
      logger.info({ userId }, 'OAuth token detected, converting to API key')
      const apiKey = await this.createApiKeyFromOAuthToken(input)
      if (apiKey) {
        tokenToStore = apiKey
        authMethod = 'api_key'
        logger.info({ userId }, 'Successfully converted OAuth token to API key')
      } else {
        authMethod = 'oauth'
        logger.warn({ userId }, 'Failed to create API key, storing OAuth token directly')
      }
    } else {
      logger.info({ userId }, 'Treating input as PAT or API key')
    }

    await this.tokenRepository.saveProviderToken(userId, 'claude', {
      authMethod,
      accessToken: tokenToStore,
      ...(refreshToken && { refreshToken }),
      tokenType: 'Bearer',
      connectedAt: new Date(),
      ...(expiresAt && { expiresAt }),
    })

    logger.info({ userId, authMethod }, 'Claude token saved successfully')
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
