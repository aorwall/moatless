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
