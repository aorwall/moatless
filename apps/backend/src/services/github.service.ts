import type {
  GitHubConfigResponse,
  GitHubOAuthStateData,
  GitHubOAuthUrlResponse,
  GitHubServiceConfig,
} from '@moatless/types'
import { inject, singleton } from 'tsyringe'
import { SessionContext } from '../features/auth/index.js'
import type { GitHubClient } from '../infrastructure/clients/github.client.js'
import { logger } from '../utils/logger.js'

interface GitHubUser {
  id: number
  login: string
  email: string | null
  name: string | null
  avatar_url: string | null
}

interface GitHubEmail {
  email: string
  primary: boolean
  verified: boolean
  visibility: string | null
}

@singleton()
export class GitHubService {
  private readonly oauthClientId: string | undefined
  private readonly oauthClientSecret: string | undefined
  private readonly serviceCallbackUrl: string | undefined

  constructor(
    @inject('GitHubClient') private client: GitHubClient,
    @inject('Storage')
    private storage: {
      save: (key: string, data: unknown) => Promise<void>
      get: <T = unknown>(key: string) => Promise<T | null>
      delete: (key: string) => Promise<void>
    },
  ) {
    // OAuth configuration for service connection
    this.oauthClientId = process.env['GITHUB_CLIENT_ID']
    this.oauthClientSecret = process.env['GITHUB_CLIENT_SECRET']
    this.serviceCallbackUrl = process.env['GITHUB_SERVICE_CALLBACK_URL']
  }

  /**
   * Fetch the authenticated user's information
   * Delegates to GitHubClient
   */
  async getAuthenticatedUser(token: string): Promise<GitHubUser | null> {
    return await this.client.getAuthenticatedUser(token)
  }

  /**
   * Fetch the authenticated user's email addresses
   * Delegates to GitHubClient
   */
  async getUserEmails(token: string): Promise<GitHubEmail[] | null> {
    return await this.client.getUserEmails(token)
  }

  /**
   * Validate a GitHub personal access token
   * Delegates to GitHubClient
   */
  async validateToken(token: string): Promise<boolean> {
    return await this.client.validateToken(token)
  }

  /**
   * Get the scopes granted to a token
   * Delegates to GitHubClient
   */
  async getTokenScopes(token: string): Promise<string[] | null> {
    return await this.client.getTokenScopes(token)
  }

  /**
   * Get repositories accessible by the authenticated user
   * Delegates to GitHubClient
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
   * Get file content from a GitHub repository
   * Delegates to GitHubClient
   * @param token GitHub personal access token
   * @param owner Repository owner
   * @param repo Repository name
   * @param path Path to the file in the repository
   * @param branch Optional branch name (defaults to default branch)
   * @returns File content as string, or null if file doesn't exist or on error
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

  /**
   * Get GitHub token for git operations with priority resolution
   * Priority: 1) GitHub OAuth session token, 2) Service connection token
   * @returns Token or null if not available
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

    // Priority 2: Check for service connection token
    const serviceConfig = await this.getServiceToken(userId)
    if (serviceConfig) {
      logger.debug('Using GitHub service connection token for git operations', {
        userId,
        method: serviceConfig.authMethod,
      })
      return serviceConfig.accessToken
    }

    logger.debug('No GitHub token available for git operations', { userId })
    return null
  }

  /**
   * Get GitHub connection configuration for current user
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

    // Check GitHub OAuth session
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

    // Check service connection
    const serviceConfig = await this.getServiceToken(userId)
    if (serviceConfig) {
      return {
        connected: true,
        oauthEnabled: this.isServiceOAuthEnabled(),
        method: serviceConfig.authMethod === 'oauth' ? 'service-oauth' : 'pat',
        githubLogin: serviceConfig.githubLogin,
        githubUserId: serviceConfig.githubUserId,
        connectedAt: serviceConfig.connectedAt,
        expiresAt: serviceConfig.expiresAt,
      }
    }

    return {
      connected: false,
      oauthEnabled: this.isServiceOAuthEnabled(),
    }
  }

  /**
   * Generate GitHub OAuth URL for service connection
   * @param userId User ID initiating OAuth
   * @returns OAuth URL and state parameter
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

    // Store state temporarily (10 minute expiration)
    const stateData: GitHubOAuthStateData = {
      state,
      userId,
      createdAt: new Date().toISOString(),
    }
    await this.storage.save(`github-service-oauth-states/${state}`, stateData)

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
   * Exchange OAuth code for service access token
   * @param code Authorization code from GitHub
   * @param state State parameter from OAuth callback
   * @param userId User ID making the request
   */
  async exchangeServiceOAuthToken(code: string, state: string, userId: string): Promise<void> {
    if (!(this.oauthClientId && this.oauthClientSecret)) {
      throw new Error('GitHub service OAuth not configured')
    }

    // Retrieve and validate state
    const stateData = await this.storage.get<GitHubOAuthStateData>(
      `github-service-oauth-states/${state}`,
    )
    if (!stateData) {
      throw new Error('Invalid or expired OAuth state')
    }

    // Validate state matches and user matches
    if (stateData.state !== state || stateData.userId !== userId) {
      throw new Error('OAuth state mismatch')
    }

    // Check expiration (10 minutes)
    const createdAt = new Date(stateData.createdAt).getTime()
    const now = Date.now()
    if (now - createdAt > 600000) {
      await this.storage.delete(`github-service-oauth-states/${state}`)
      throw new Error('OAuth code expired')
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
      const user = await this.getAuthenticatedUser(tokenResponse.access_token)

      // Save service token
      const serviceConfig: GitHubServiceConfig = {
        userId,
        authMethod: 'oauth',
        accessToken: tokenResponse.access_token,
        tokenType: tokenResponse.token_type,
        scope: tokenResponse.scope,
        connectedAt: new Date().toISOString(),
        githubUserId: user?.id.toString(),
        githubLogin: user?.login,
      }

      await this.storage.save(`users/${userId}/github-service`, serviceConfig)

      // Clean up state
      await this.storage.delete(`github-service-oauth-states/${state}`)

      logger.info('GitHub service OAuth token saved successfully', {
        userId,
        githubUserId: user?.id,
        githubLogin: user?.login,
      })
    } catch (error) {
      // Clean up state on error
      await this.storage.delete(`github-service-oauth-states/${state}`)
      throw error
    }
  }

  /**
   * Save Personal Access Token for service connection
   * @param userId User ID
   * @param accessToken GitHub Personal Access Token
   */
  async saveServicePAT(userId: string, accessToken: string): Promise<void> {
    // Validate token by fetching user info
    const user = await this.getAuthenticatedUser(accessToken)
    if (!user) {
      throw new Error('Invalid GitHub token')
    }

    const serviceConfig: GitHubServiceConfig = {
      userId,
      authMethod: 'pat',
      accessToken,
      connectedAt: new Date().toISOString(),
      githubUserId: user.id.toString(),
      githubLogin: user.login,
    }

    await this.storage.save(`users/${userId}/github-service`, serviceConfig)

    logger.info('GitHub service PAT saved successfully', {
      userId,
      githubUserId: user.id,
      githubLogin: user.login,
    })
  }

  /**
   * Get user's service connection token
   * @param userId User ID
   * @returns Service configuration or null
   */
  async getServiceToken(userId: string): Promise<GitHubServiceConfig | null> {
    return await this.storage.get<GitHubServiceConfig>(`users/${userId}/github-service`)
  }

  /**
   * Delete user's service connection token
   * @param userId User ID
   */
  async deleteServiceToken(userId: string): Promise<void> {
    await this.storage.delete(`users/${userId}/github-service`)
    logger.info('GitHub service token deleted', { userId })
  }

  /**
   * Check if service OAuth is enabled/configured
   * @returns true if GITHUB_CLIENT_ID and GITHUB_SERVICE_CALLBACK_URL are set
   */
  isServiceOAuthEnabled(): boolean {
    return !!(this.oauthClientId && this.serviceCallbackUrl)
  }
}

/**
 * @deprecated Use TSyringe container.resolve(GitHubService) instead
 * Temporary factory function for backward compatibility during migration
 */
export function createGitHubService(
  client: GitHubClient,
  storage: {
    save: (key: string, data: unknown) => Promise<void>
    get: <T = unknown>(key: string) => Promise<T | null>
    delete: (key: string) => Promise<void>
  },
): GitHubService {
  return new GitHubService(client, storage)
}

// Legacy singleton instance (for backwards compatibility during migration)
let githubServiceInstance: GitHubService | null = null

export function getGitHubService(): GitHubService {
  if (!githubServiceInstance) {
    const { createGitHubClient } = require('../infrastructure/clients/github.client.js')
    // Create without dependencies for legacy code
    // Note: This will not have service OAuth or token resolution capabilities
    githubServiceInstance = new GitHubService(createGitHubClient(), {
      save: async () => {
        /* No-op for legacy singleton */
      },
      get: async () => null,
      delete: async () => {
        /* No-op for legacy singleton */
      },
    })
  }
  return githubServiceInstance
}

// For testing: reset the singleton instance
export function resetGitHubServiceInstance(): void {
  githubServiceInstance = null
}
