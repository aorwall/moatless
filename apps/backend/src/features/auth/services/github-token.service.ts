import type { GitHubConfigResponse, GitHubOAuthUrlResponse } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { GitHubClient } from '../../../infrastructure/clients/github.client'
import { GitHubClientSymbol } from '../../../infrastructure/clients/github.client'
import { logger } from '../../../utils/logger'
import { OAuthStateRepository } from '../db/oauth-state.repository'
import type { ProviderTokenRepository } from '../db/provider-token.repository'
import { SessionContext } from './session-context'

/**
 * GitHub Token Service
 *
 * Manages GitHub tokens for git operations with unified resolution strategy.
 * Consolidates token management from the legacy GitHubService into the auth feature.
 *
 * Token Resolution Priority:
 * 1. Session token (if user authenticated via GitHub OAuth with 'repo' scope)
 * 2. Linked provider token (PAT or service OAuth stored in provider_tokens table)
 * 3. null
 *
 * Use Cases:
 * - User logs in via GitHub OAuth → session token automatically works for git ops
 * - User logs in via Auth0/other → can link GitHub via PAT or service OAuth
 * - User prefers PAT → can use PAT instead of OAuth
 */
@injectable()
export class GitHubTokenService {
  private readonly oauthClientId: string | undefined
  private readonly oauthClientSecret: string | undefined
  private readonly serviceCallbackUrl: string | undefined

  constructor(
    @inject(GitHubClientSymbol) private readonly client: GitHubClient,
    @inject('ProviderTokenRepository') private readonly tokenRepository: ProviderTokenRepository,
    @inject(OAuthStateRepository) private readonly stateRepository: OAuthStateRepository,
  ) {
    // OAuth configuration for service connection (separate from auth OAuth)
    this.oauthClientId = process.env['GITHUB_CLIENT_ID']
    this.oauthClientSecret = process.env['GITHUB_CLIENT_SECRET']
    this.serviceCallbackUrl = process.env['GITHUB_SERVICE_CALLBACK_URL']
  }

  /**
   * Get GitHub token for git operations with unified priority resolution.
   *
   * Priority:
   * 1. GitHub OAuth session token (if user logged in via GitHub with repo scope)
   * 2. Service connection token (PAT or service OAuth from provider_tokens table)
   * 3. null (no token available)
   *
   * @returns Token string or null if not available
   */
  async getTokenForGitOperations(): Promise<string | null> {
    const userId = SessionContext.getCurrentUserId()
    if (!userId) {
      logger.debug('No authenticated user for git operations')
      return null
    }

    // Priority 1: Check if user is authenticated via GitHub OAuth
    const session = SessionContext.getCurrentSession()
    if (session?.provider === 'github' && session.accessToken) {
      // Verify session has repo scope
      if (session.scope?.includes('repo')) {
        logger.debug('Using GitHub OAuth session token for git operations', { userId })
        return session.accessToken
      }
      logger.warn('GitHub OAuth session missing repo scope', { userId, scope: session.scope })
    }

    // Priority 2: Check for linked provider token (PAT or service OAuth)
    const providerToken = await this.tokenRepository.getProviderToken(userId, 'github')
    if (providerToken) {
      logger.debug('Using linked GitHub provider token for git operations', {
        userId,
        method: providerToken.authMethod,
      })
      return providerToken.accessToken
    }

    logger.debug('No GitHub token available for git operations', { userId })
    return null
  }

  /**
   * Get GitHub connection configuration for current user.
   *
   * Returns connection status, method, and metadata about the GitHub connection.
   * Used by the settings UI to display current connection state.
   *
   * @returns Configuration with connection status and method
   */
  async getConnectionConfig(): Promise<GitHubConfigResponse> {
    const userId = SessionContext.getCurrentUserId()
    if (!userId) {
      return {
        connected: false,
        oauthEnabled: this.isServiceOAuthEnabled(),
      }
    }

    // Check GitHub OAuth session (primary authentication)
    const session = SessionContext.getCurrentSession()
    if (session?.provider === 'github' && session.accessToken) {
      return {
        connected: true,
        oauthEnabled: this.isServiceOAuthEnabled(),
        method: 'github-oauth',
        githubLogin: session.userLogin,
        githubUserId: session.userId,
      }
    }

    // Check linked provider token (PAT or service OAuth)
    const providerToken = await this.tokenRepository.getProviderToken(userId, 'github')
    if (providerToken) {
      return {
        connected: true,
        oauthEnabled: this.isServiceOAuthEnabled(),
        method: providerToken.authMethod === 'oauth' ? 'service-oauth' : 'pat',
        githubLogin: providerToken.githubLogin,
        githubUserId: providerToken.githubUserId,
        connectedAt: providerToken.connectedAt,
        expiresAt: providerToken.expiresAt,
      }
    }

    return {
      connected: false,
      oauthEnabled: this.isServiceOAuthEnabled(),
    }
  }

  /**
   * Save Personal Access Token for GitHub service connection.
   *
   * Validates the token by fetching user info, then stores it in the database.
   * Used when users prefer PAT over OAuth or are logged in via non-GitHub provider.
   *
   * @param userId - User identifier
   * @param accessToken - GitHub Personal Access Token
   * @throws Error if token is invalid
   */
  async saveProviderPAT(userId: string, accessToken: string): Promise<void> {
    // Validate token by fetching user info
    const user = await this.client.getAuthenticatedUser(accessToken)
    if (!user) {
      throw new Error('Invalid GitHub token')
    }

    await this.tokenRepository.saveProviderToken(userId, 'github', {
      authMethod: 'pat',
      accessToken,
      connectedAt: new Date(),
      providerUserId: user.id.toString(),
      providerLogin: user.login,
    })

    logger.info('GitHub PAT saved successfully', {
      userId,
      githubUserId: user.id,
      githubLogin: user.login,
    })
  }

  /**
   * Delete user's GitHub provider token.
   *
   * Removes the linked PAT or service OAuth token from the database.
   * Does not affect GitHub OAuth session (primary authentication).
   *
   * @param userId - User identifier
   */
  async deleteProviderToken(userId: string): Promise<void> {
    await this.tokenRepository.deleteProviderToken(userId, 'github')
    logger.info('GitHub provider token deleted', { userId })
  }

  /**
   * Generate GitHub OAuth URL for service connection.
   *
   * Creates a separate OAuth flow for linking GitHub to non-GitHub authenticated users.
   * Different from the primary GitHub OAuth authentication flow.
   *
   * @param userId - User ID initiating OAuth
   * @returns OAuth URL and state parameter
   * @throws Error if OAuth is not configured
   */
  async generateServiceOAuthUrl(userId: string): Promise<GitHubOAuthUrlResponse> {
    if (!(this.oauthClientId && this.serviceCallbackUrl)) {
      throw new Error(
        'GitHub service OAuth not configured. Missing GITHUB_CLIENT_ID or GITHUB_SERVICE_CALLBACK_URL',
      )
    }

    const { randomBytes } = await import('node:crypto')

    // Generate random state for CSRF protection
    const state = randomBytes(32).toString('hex')

    // Store state in oauth_states table for CSRF protection (expires in 10 minutes)
    const expiresAt = new Date(Date.now() + 600000) // 10 minutes from now
    await this.stateRepository.saveState(state, userId, 'github', expiresAt)

    // Build OAuth URL
    const scope = process.env['GITHUB_OAUTH_SCOPE'] || 'user:email repo'
    const params = new URLSearchParams({
      client_id: this.oauthClientId,
      redirect_uri: this.serviceCallbackUrl,
      scope,
      state,
    })

    const authUrl = `https://github.com/login/oauth/authorize?${params.toString()}`

    logger.info('Generated GitHub service OAuth URL', { userId, state })

    return { authUrl, state }
  }

  /**
   * Exchange OAuth code for service access token.
   *
   * Completes the service OAuth flow by exchanging the authorization code for an access token.
   * Stores the token in provider_tokens table.
   *
   * @param code - Authorization code from GitHub
   * @param state - State parameter from OAuth callback
   * @param userId - User ID making the request
   * @throws Error if state is invalid, expired, or token exchange fails
   */
  async exchangeServiceOAuthToken(code: string, state: string, userId: string): Promise<void> {
    if (!(this.oauthClientId && this.oauthClientSecret)) {
      throw new Error('GitHub service OAuth not configured')
    }

    // Retrieve and validate state
    const oauthState = await this.stateRepository.getState(state)
    if (!oauthState) {
      throw new Error('Invalid or expired OAuth state')
    }

    // Validate user matches
    if (oauthState.userId !== userId) {
      throw new Error('OAuth state mismatch')
    }

    try {
      // Exchange code for token
      const response = await fetch('https://github.com/login/oauth/access_token', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json',
        },
        body: JSON.stringify({
          client_id: this.oauthClientId,
          client_secret: this.oauthClientSecret,
          code,
        }),
      })

      if (!response.ok) {
        const errorText = await response.text()
        logger.error(
          'GitHub service OAuth token exchange failed',
          new Error(`Status: ${response.status}`),
          {
            status: response.status,
            error: errorText,
          },
        )
        throw new Error('Failed to exchange OAuth code for token')
      }

      const tokenResponse = (await response.json()) as {
        access_token: string
        token_type: string
        scope: string
        error?: string
      }

      if (tokenResponse.error) {
        throw new Error(tokenResponse.error)
      }

      // Fetch user info to get GitHub user ID and login
      const user = await this.client.getAuthenticatedUser(tokenResponse.access_token)

      // Save provider token
      const tokenData: {
        authMethod: 'oauth'
        accessToken: string
        tokenType?: string
        scope?: string
        connectedAt: Date
        providerUserId?: string
        providerLogin?: string
      } = {
        authMethod: 'oauth',
        accessToken: tokenResponse.access_token,
        connectedAt: new Date(),
      }

      if (tokenResponse.token_type) {
        tokenData.tokenType = tokenResponse.token_type
      }
      if (tokenResponse.scope) {
        tokenData.scope = tokenResponse.scope
      }
      if (user) {
        tokenData.providerUserId = user.id.toString()
        tokenData.providerLogin = user.login
      }

      await this.tokenRepository.saveProviderToken(userId, 'github', tokenData)

      // Clean up state
      await this.stateRepository.deleteState(state)

      logger.info('GitHub service OAuth token saved successfully', {
        userId,
        githubUserId: user?.id,
        githubLogin: user?.login,
      })
    } catch (error) {
      // Clean up state on error
      await this.stateRepository.deleteState(state)
      throw error
    }
  }

  /**
   * Check if service OAuth is enabled/configured.
   *
   * @returns true if GITHUB_CLIENT_ID and GITHUB_SERVICE_CALLBACK_URL are set
   */
  isServiceOAuthEnabled(): boolean {
    return !!(this.oauthClientId && this.serviceCallbackUrl)
  }

  /**
   * Get the scopes granted to a token.
   * Delegates to GitHubClient.
   */
  async getTokenScopes(token: string): Promise<string[] | null> {
    return await this.client.getTokenScopes(token)
  }

  /**
   * Validate a GitHub token.
   * Delegates to GitHubClient.
   */
  async validateToken(token: string): Promise<boolean> {
    return await this.client.validateToken(token)
  }

  /**
   * Get repositories accessible by the authenticated user.
   * Delegates to GitHubClient.
   */
  async getUserRepositories(
    token: string,
    options?: { page?: number; per_page?: number },
  ): Promise<
    | {
        id: number
        name: string
        full_name: string
        private: boolean
      }[]
    | null
  > {
    return await this.client.getUserRepositories(token, options)
  }

  /**
   * Get file content from a GitHub repository.
   * Delegates to GitHubClient.
   */
  async getFileContent(
    token: string,
    owner: string,
    repo: string,
    path: string,
    branch?: string,
  ): Promise<string | null> {
    return await this.client.getFileContent(token, owner, repo, path, branch)
  }
}
