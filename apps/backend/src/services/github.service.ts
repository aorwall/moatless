import type {
  GitHubConfigResponse,
  GitHubOAuthStateData,
  GitHubOAuthUrlResponse,
  GitHubServiceConfig,
  UserSession,
} from '@moatless/types'
import { SessionContext } from '../services/access-control/context-session-provider.js'
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

interface GitHubApiUserResponse {
  id: number
  login: string
  email: string | null
  name: string | null
  avatar_url: string | null
}

export class GitHubService {
  private readonly apiBaseUrl = 'https://api.github.com'
  private readonly oauthClientId: string | undefined
  private readonly oauthClientSecret: string | undefined
  private readonly serviceCallbackUrl: string | undefined

  constructor(
    private storage: {
      save: (key: string, data: unknown) => Promise<void>
      get: <T = unknown>(key: string) => Promise<T | null>
      delete: (key: string) => Promise<void>
    },
    private authService?: {
      getSession: (sessionId: string) => Promise<UserSession | null>
    },
  ) {
    // OAuth configuration for service connection
    this.oauthClientId = process.env['GITHUB_CLIENT_ID']
    this.oauthClientSecret = process.env['GITHUB_CLIENT_SECRET']
    this.serviceCallbackUrl = process.env['GITHUB_SERVICE_CALLBACK_URL']
  }

  /**
   * Fetch the authenticated user's information
   */
  async getAuthenticatedUser(token: string): Promise<GitHubUser | null> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user`, {
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github.v3+json',
        },
      })

      if (!response.ok) {
        if (response.status === 401) {
          logger.error('GitHub API: Invalid or expired token')
        } else {
          logger.error('GitHub API: Failed to fetch user', new Error(`Status: ${response.status}`))
        }
        return null
      }

      const userData = (await response.json()) as GitHubApiUserResponse

      // GitHub API returns email as null if it's private
      // We need to fetch the primary email separately
      let email = userData.email

      if (!email) {
        const userEmails = await this.getUserEmails(token)
        if (userEmails) {
          const primaryEmail = userEmails.find((e) => e.primary && e.verified)
          if (primaryEmail) {
            email = primaryEmail.email
          }
        }
      }

      return {
        id: userData.id,
        login: userData.login,
        email: email,
        name: userData.name,
        avatar_url: userData.avatar_url,
      }
    } catch (error) {
      logger.error('Error fetching GitHub user', error as Error)
      return null
    }
  }

  /**
   * Fetch the authenticated user's email addresses
   */
  async getUserEmails(token: string): Promise<GitHubEmail[] | null> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user/emails`, {
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github.v3+json',
        },
      })

      if (!response.ok) {
        logger.error(
          'GitHub API: Failed to fetch user emails',
          new Error(`Status: ${response.status}`),
        )
        return null
      }

      return (await response.json()) as GitHubEmail[]
    } catch (error) {
      logger.error('Error fetching GitHub user emails', error as Error)
      return null
    }
  }

  /**
   * Validate a GitHub personal access token
   */
  async validateToken(token: string): Promise<boolean> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user`, {
        method: 'HEAD',
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github.v3+json',
        },
      })

      return response.ok
    } catch (error) {
      logger.error('Error validating GitHub token', error as Error)
      return false
    }
  }

  /**
   * Get the scopes granted to a token
   */
  async getTokenScopes(token: string): Promise<string[] | null> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user`, {
        method: 'HEAD',
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github.v3+json',
        },
      })

      if (!response.ok) {
        return null
      }

      // GitHub returns scopes in the X-OAuth-Scopes header
      const scopesHeader = response.headers.get('X-OAuth-Scopes')
      if (!scopesHeader) {
        return []
      }

      return scopesHeader.split(',').map((scope) => scope.trim())
    } catch (error) {
      logger.error('Error fetching GitHub token scopes', error as Error)
      return null
    }
  }

  /**
   * Get repositories accessible by the authenticated user
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
    try {
      const page = options?.page || 1
      const perPage = options?.per_page || 30

      const response = await fetch(
        `${this.apiBaseUrl}/user/repos?page=${page}&per_page=${perPage}&sort=updated`,
        {
          headers: {
            Authorization: `Bearer ${token}`,
            Accept: 'application/vnd.github.v3+json',
          },
        },
      )

      if (!response.ok) {
        logger.error(
          'GitHub API: Failed to fetch user repositories',
          new Error(`Status: ${response.status}`),
        )
        return null
      }

      const repos = (await response.json()) as {
        id: number
        name: string
        full_name: string
        private: boolean
      }[]

      return repos
    } catch (error) {
      logger.error('Error fetching GitHub user repositories', error as Error)
      return null
    }
  }

  /**
   * Get file content from a GitHub repository
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
    try {
      const url = `${this.apiBaseUrl}/repos/${owner}/${repo}/contents/${path}${branch ? `?ref=${branch}` : ''}`

      const response = await fetch(url, {
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github.v3+json',
        },
      })

      if (!response.ok) {
        if (response.status === 404) {
          logger.debug('GitHub API: File not found', { owner, repo, path, branch })
          return null
        }
        logger.error(
          'GitHub API: Failed to fetch file content',
          new Error(`Status: ${response.status}`),
        )
        return null
      }

      const data = (await response.json()) as {
        content?: string
        encoding?: string
        type: string
      }

      // GitHub API returns files with type 'file' and base64 encoded content
      if (data.type !== 'file' || !data.content || data.encoding !== 'base64') {
        logger.error('GitHub API: Unexpected response format for file content')
        return null
      }

      // Decode base64 content
      const content = Buffer.from(data.content, 'base64').toString('utf-8')
      return content
    } catch (error) {
      logger.error('Error fetching file content from GitHub', error as Error)
      return null
    }
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
    const sessionId = SessionContext.getCurrentSessionId()
    if (sessionId && this.authService) {
      const session = await this.authService.getSession(sessionId)
      if (session?.provider === 'github' && session.accessToken) {
        // Verify session has repo scope
        if (session.scope?.includes('repo')) {
          logger.debug('Using GitHub OAuth session token for git operations', { userId })
          return session.accessToken
        }
        logger.warn('GitHub OAuth session missing repo scope', { userId, scope: session.scope })
      }
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
    const sessionId = SessionContext.getCurrentSessionId()
    if (sessionId && this.authService) {
      const session = await this.authService.getSession(sessionId)
      if (session?.provider === 'github' && session.accessToken) {
        return {
          connected: true,
          oauthEnabled: this.isServiceOAuthEnabled(),
          method: 'github-oauth',
          githubLogin: session.userLogin,
          githubUserId: session.userId,
        }
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
 * Factory function to create GitHubService with dependencies
 */
export function createGitHubService(
  storage: {
    save: (key: string, data: unknown) => Promise<void>
    get: <T = unknown>(key: string) => Promise<T | null>
    delete: (key: string) => Promise<void>
  },
  authService?: {
    getSession: (sessionId: string) => Promise<UserSession | null>
  },
): GitHubService {
  return new GitHubService(storage, authService)
}

// Legacy singleton instance (for backwards compatibility during migration)
let githubServiceInstance: GitHubService | null = null

export function getGitHubService(): GitHubService {
  if (!githubServiceInstance) {
    // Create without dependencies for legacy code
    // Note: This will not have service OAuth or token resolution capabilities
    githubServiceInstance = new GitHubService(
      {
        save: async () => {
          /* No-op for legacy singleton */
        },
        get: async () => null,
        delete: async () => {
          /* No-op for legacy singleton */
        },
      },
      undefined,
    )
  }
  return githubServiceInstance
}

// For testing: reset the singleton instance
export function resetGitHubServiceInstance(): void {
  githubServiceInstance = null
}
