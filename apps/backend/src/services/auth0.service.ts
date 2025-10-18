import type { Auth0User } from '@moatless/types'
import { decodeJwt } from 'jose'
import { logger } from '../utils/logger.js'

/**
 * Auth0 Service
 * Handles Auth0 OAuth and API interactions
 */
export class Auth0Service {
  private readonly domain: string
  private readonly groupsClaim: string

  constructor() {
    this.domain = process.env['AUTH0_DOMAIN'] || ''
    this.groupsClaim = process.env['AUTH0_GROUPS_CLAIM'] || 'https://moatless.ai/groups'
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

  /**
   * Check if Auth0 is properly configured
   */
  isConfigured(): boolean {
    return !!(
      process.env['AUTH0_DOMAIN'] &&
      process.env['AUTH0_CLIENT_ID'] &&
      process.env['AUTH0_CLIENT_SECRET']
    )
  }

  /**
   * Decode Auth0 ID token and extract claims
   * @param idToken - The Auth0 ID token (JWT)
   * @returns Decoded token payload or null if decode fails
   */
  decodeIdToken(idToken: string): Record<string, unknown> | null {
    try {
      const payload = decodeJwt(idToken)
      return payload as Record<string, unknown>
    } catch (error) {
      logger.error('Failed to decode Auth0 ID token', error as Error)
      return null
    }
  }

  /**
   * Extract groups from decoded ID token
   * @param decodedToken - The decoded ID token payload
   * @returns Array of group IDs the user belongs to
   */
  extractGroups(decodedToken: Record<string, unknown>): string[] {
    try {
      const groupsClaim = decodedToken[this.groupsClaim]

      // Handle different possible group claim formats
      if (Array.isArray(groupsClaim)) {
        // Groups as array of strings
        return groupsClaim.filter((group): group is string => typeof group === 'string')
      }

      if (typeof groupsClaim === 'string') {
        // Single group as string
        return [groupsClaim]
      }

      // No groups claim found
      logger.debug(`No groups claim found at ${this.groupsClaim}`)
      return []
    } catch (error) {
      logger.error('Error extracting groups from Auth0 token', error as Error)
      return []
    }
  }

  /**
   * Get user's primary group from ID token
   * Returns the first group if user belongs to multiple groups
   * @param idToken - The Auth0 ID token
   * @returns Primary group ID or null if no groups found
   */
  getPrimaryGroup(idToken: string): string | null {
    const decoded = this.decodeIdToken(idToken)
    if (!decoded) {
      return null
    }

    const groups = this.extractGroups(decoded)
    if (groups.length === 0) {
      return null
    }

    // Return first group as primary
    const primaryGroup = groups[0]
    if (!primaryGroup) {
      return null
    }

    if (groups.length > 1) {
      logger.info(`User has ${groups.length} groups, using first as primary: ${primaryGroup}`)
    }

    return primaryGroup
  }

  /**
   * Generate Auth0 logout URL
   * @param returnTo - URL to redirect to after logout
   * @returns Auth0 logout URL or null if not configured
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
 * Factory function to create Auth0Service
 */
export function createAuth0Service(): Auth0Service {
  return new Auth0Service()
}
