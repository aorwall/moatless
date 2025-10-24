import { singleton } from 'tsyringe'
import type { GitHubClient } from '../../../infrastructure/clients/github.client.js'
import { logger } from '../../../utils/logger.js'
import type { OAuthTokens, ProviderUserInfo } from '../types/oauth.types.js'
import type { OAuthProvider } from './oauth-provider.interface.js'

/**
 * GitHub OAuth Provider Implementation
 *
 * Implements OAuth flow for GitHub authentication.
 * Note: GitHub does NOT support group-based access control via OAuth,
 * so extractUserGroup is not implemented.
 */
@singleton()
export class GitHubOAuthProvider implements OAuthProvider {
  constructor(private readonly client: GitHubClient) {}

  getProviderId(): string {
    return 'github'
  }

  generateAuthUrl(state: string): string | null {
    const clientId = process.env['GITHUB_CLIENT_ID']
    const callbackUrl = process.env['GITHUB_CALLBACK_URL']

    if (!(clientId && callbackUrl)) {
      logger.error('GitHub OAuth credentials not configured')
      return null
    }

    // Get OAuth scope from environment or use default
    // Default includes repo scope to enable repository cloning
    const scope = process.env['GITHUB_OAUTH_SCOPE'] || 'user:email repo'

    const params = new URLSearchParams({
      client_id: clientId,
      redirect_uri: callbackUrl,
      state,
      scope,
    })

    return `https://github.com/login/oauth/authorize?${params.toString()}`
  }

  async exchangeCodeForTokens(code: string): Promise<OAuthTokens> {
    const clientId = process.env['GITHUB_CLIENT_ID']
    const clientSecret = process.env['GITHUB_CLIENT_SECRET']

    if (!(clientId && clientSecret)) {
      throw new Error('GitHub OAuth credentials not configured')
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
        throw new Error(`Failed to exchange code for token: ${response.status}`)
      }

      const data = (await response.json()) as {
        access_token: string
        token_type: string
        scope: string
        error?: string
      }

      if (data.error) {
        throw new Error(`GitHub OAuth error: ${data.error}`)
      }

      return {
        accessToken: data.access_token,
        tokenType: data.token_type,
        scope: data.scope,
      }
    } catch (error) {
      logger.error('Error exchanging GitHub code for token', error as Error)
      throw error
    }
  }

  async getUserInfo(tokens: OAuthTokens): Promise<ProviderUserInfo> {
    const user = await this.client.getAuthenticatedUser(tokens.accessToken)

    if (!user) {
      throw new Error('Failed to fetch GitHub user information')
    }

    return {
      id: String(user.id),
      login: user.login,
      email: user.email,
      name: user.name,
      avatarUrl: user.avatar_url,
    }
  }

  // GitHub does NOT support group extraction - method intentionally omitted
  // GitHub does NOT require custom logout URL - method intentionally omitted
}

/**
 * Factory function for creating GitHub OAuth provider
 * Used by app.ts to create provider instances for the OAuth provider map
 */
export function createGitHubOAuthProvider(client: GitHubClient): GitHubOAuthProvider {
  return new GitHubOAuthProvider(client)
}
