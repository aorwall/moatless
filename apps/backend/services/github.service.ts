import { logger } from '../utils/logger.js'

export interface GitHubUser {
  login: string
  email: string | null
  name: string | null
}

export interface GitHubEmail {
  email: string
  primary: boolean
  verified: boolean
  visibility: string | null
}

export class GitHubService {
  private readonly apiBaseUrl = 'https://api.github.com'

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

      const userData = (await response.json()) as any

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
        login: userData.login,
        email: email,
        name: userData.name,
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
}

// Singleton instance
let githubServiceInstance: GitHubService | null = null

export function getGitHubService(): GitHubService {
  if (!githubServiceInstance) {
    githubServiceInstance = new GitHubService()
  }
  return githubServiceInstance
}

// For testing: reset the singleton instance
export function resetGitHubServiceInstance(): void {
  githubServiceInstance = null
}
