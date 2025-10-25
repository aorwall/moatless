import type { ServiceIdentifier } from 'inversify'
import { logger } from '../../utils/logger.js'

/**
 * User information from Auth0
 */
export interface Auth0User {
  sub: string // Auth0 user ID (e.g., "auth0|123456")
  email: string | null
  name: string | null
  nickname: string | null
  picture: string | null
  email_verified?: boolean | undefined
  // Allow custom claims (namespaced properties like "https://moatless.ai/groups")
  [key: string]: unknown
}

/**
 * DI symbol for Auth0Client.
 * Use this symbol to inject Auth0 client implementations.
 */
export const Auth0ClientSymbol: ServiceIdentifier<Auth0Client> = Symbol.for('Auth0Client')

/**
 * Pure Auth0 API client for making HTTP requests to Auth0's API
 * Contains no business logic - only API communication
 */
export class Auth0Client {
  private readonly domain: string

  constructor() {
    this.domain = process.env['AUTH0_DOMAIN'] || ''
  }

  /**
   * Get authenticated user information from Auth0
   */
  async getAuthenticatedUser(accessToken: string): Promise<Auth0User | null> {
    if (!this.domain) {
      logger.error('AUTH0_DOMAIN not configured')
      return null
    }

    try {
      const response = await fetch(`https://${this.domain}/userinfo`, {
        headers: {
          Authorization: `Bearer ${accessToken}`,
        },
      })

      if (!response.ok) {
        logger.error('Failed to fetch Auth0 user info', new Error(`Status: ${response.status}`))
        return null
      }

      const user = (await response.json()) as Auth0User

      return {
        sub: user.sub,
        email: user.email || null,
        name: user.name || null,
        nickname: user.nickname || null,
        picture: user.picture || null,
        email_verified: user.email_verified,
      }
    } catch (error) {
      logger.error('Error fetching Auth0 user info', error as Error)
      return null
    }
  }

  /**
   * Validate Auth0 access token
   */
  async validateToken(accessToken: string): Promise<boolean> {
    try {
      const user = await this.getAuthenticatedUser(accessToken)
      return user !== null
    } catch (error) {
      logger.error('Error validating Auth0 token', error as Error)
      return false
    }
  }
}

/**
 * Factory function to create Auth0Client
 */
export function createAuth0Client(): Auth0Client {
  return new Auth0Client()
}
