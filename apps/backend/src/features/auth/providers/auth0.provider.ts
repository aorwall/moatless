import { decodeJwt } from 'jose'
import type { Auth0Client } from '../../../infrastructure/clients/auth0.client.js'
import { logger } from '../../../utils/logger.js'
import type { OAuthTokens, ProviderUserInfo } from '../types/oauth.types.js'
import type { OAuthProvider } from './oauth-provider.interface.js'

/**
 * Auth0 OAuth Provider Implementation
 *
 * Implements OAuth flow for Auth0 authentication with support for:
 * - ID tokens (OpenID Connect)
 * - Group extraction from JWT claims
 * - Custom logout flow
 */
export class Auth0OAuthProvider implements OAuthProvider {
  private readonly groupsClaim: string

  constructor(private readonly client: Auth0Client) {
    this.groupsClaim = process.env['AUTH0_GROUPS_CLAIM'] || 'https://moatless.ai/groups'
  }

  getProviderId(): string {
    return 'auth0'
  }

  generateAuthUrl(state: string): string | null {
    const domain = process.env['AUTH0_DOMAIN']
    const clientId = process.env['AUTH0_CLIENT_ID']
    const callbackUrl = process.env['AUTH0_CALLBACK_URL']
    const audience = process.env['AUTH0_AUDIENCE']

    if (!(domain && clientId && callbackUrl)) {
      logger.error('Auth0 OAuth credentials not configured')
      return null
    }

    const params = new URLSearchParams({
      response_type: 'code',
      client_id: clientId,
      redirect_uri: callbackUrl,
      state,
      scope: 'openid profile email',
    })

    if (audience) {
      params.set('audience', audience)
    }

    return `https://${domain}/authorize?${params.toString()}`
  }

  async exchangeCodeForTokens(code: string): Promise<OAuthTokens> {
    const domain = process.env['AUTH0_DOMAIN']
    const clientId = process.env['AUTH0_CLIENT_ID']
    const clientSecret = process.env['AUTH0_CLIENT_SECRET']
    const callbackUrl = process.env['AUTH0_CALLBACK_URL']

    if (!(domain && clientId && clientSecret && callbackUrl)) {
      throw new Error('Auth0 OAuth credentials not configured')
    }

    try {
      const response = await fetch(`https://${domain}/oauth/token`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          grant_type: 'authorization_code',
          client_id: clientId,
          client_secret: clientSecret,
          code,
          redirect_uri: callbackUrl,
        }),
      })

      if (!response.ok) {
        throw new Error(`Failed to exchange Auth0 code for token: ${response.status}`)
      }

      const data = (await response.json()) as {
        access_token: string
        id_token: string
        token_type: string
        expires_in: number
        scope?: string
        error?: string
      }

      if (data.error) {
        throw new Error(`Auth0 OAuth error: ${data.error}`)
      }

      return {
        accessToken: data.access_token,
        idToken: data.id_token,
        tokenType: data.token_type,
        scope: data.scope,
      }
    } catch (error) {
      logger.error('Error exchanging Auth0 code for token', error as Error)
      throw error
    }
  }

  async getUserInfo(tokens: OAuthTokens): Promise<ProviderUserInfo> {
    const user = await this.client.getAuthenticatedUser(tokens.accessToken)

    if (!user) {
      throw new Error('Failed to fetch Auth0 user information')
    }

    return {
      id: user.sub,
      login: user.nickname || user.email || user.sub,
      email: user.email,
      name: user.name,
      avatarUrl: user.picture,
    }
  }

  /**
   * Extract user's group from Auth0 ID token
   * Auth0 supports group-based access control via custom claims in JWT
   */
  extractUserGroup(tokens: OAuthTokens): Promise<string | undefined> {
    if (!tokens.idToken) {
      logger.warn('No ID token provided for group extraction')
      return Promise.resolve(undefined)
    }

    try {
      const decoded = decodeJwt(tokens.idToken)
      const groupsClaim = decoded[this.groupsClaim]

      // Handle different possible group claim formats
      let groups: string[] = []

      if (Array.isArray(groupsClaim)) {
        // Groups as array of strings
        groups = groupsClaim.filter((group): group is string => typeof group === 'string')
      } else if (typeof groupsClaim === 'string') {
        // Single group as string
        groups = [groupsClaim]
      } else {
        logger.debug(`No groups claim found at ${this.groupsClaim}`)
        return Promise.resolve(undefined)
      }

      if (groups.length === 0) {
        return Promise.resolve(undefined)
      }

      // Return first group as primary
      const primaryGroup = groups[0]

      if (groups.length > 1) {
        logger.info(`User has ${groups.length} groups, using first as primary: ${primaryGroup}`)
      }

      return Promise.resolve(primaryGroup)
    } catch (error) {
      logger.error('Error extracting groups from Auth0 token', error as Error)
      return Promise.resolve(undefined)
    }
  }

  /**
   * Generate Auth0 logout URL
   * Auth0 requires server-side logout to clear the Auth0 session
   */
  generateLogoutUrl(returnTo: string): string | null {
    const domain = process.env['AUTH0_DOMAIN']
    const clientId = process.env['AUTH0_CLIENT_ID']

    if (!(domain && clientId)) {
      logger.error('Auth0 credentials not configured for logout')
      return null
    }

    const params = new URLSearchParams({
      client_id: clientId,
      returnTo,
    })

    return `https://${domain}/v2/logout?${params.toString()}`
  }
}

/**
 * Factory function to create Auth0OAuthProvider
 */
export function createAuth0OAuthProvider(client: Auth0Client): Auth0OAuthProvider {
  return new Auth0OAuthProvider(client)
}
