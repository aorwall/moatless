import type { ServiceIdentifier } from 'inversify'
import { logger } from '../../utils/logger'
import type { GitProviderClient } from './git-provider.interface'

interface GitHubBranch {
  name: string
  commit: {
    sha: string
    url: string
  }
  protected: boolean
}

interface GitHubRepository {
  id: number
  name: string
  full_name: string
  default_branch: string
  private: boolean
  html_url: string
  clone_url: string
}

interface GitHubUser {
  login: string
  id: number
  type: string
}

export interface CreateRepositoryResult {
  url: string
  cloneUrl: string
  defaultBranch: string
  fullName: string
}

/**
 * DI symbol for GitHubProviderClient.
 * Use this symbol to inject GitHub provider client implementations.
 */
export const GitHubProviderClientSymbol: ServiceIdentifier<GitHubProviderClient> =
  Symbol.for('GitHubProviderClient')

/**
 * GitHub provider client implementing the GitProviderClient interface.
 * Provides repository metadata operations via GitHub REST API.
 */
export class GitHubProviderClient implements GitProviderClient {
  private readonly apiBaseUrl = 'https://api.github.com'

  /**
   * List all branches for a GitHub repository
   */
  async listBranches(remoteUrl: string, token?: string): Promise<string[]> {
    const { owner, repo } = this.parseUrl(remoteUrl)

    try {
      const headers: Record<string, string> = {
        Accept: 'application/vnd.github.v3+json',
      }

      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }

      const response = await fetch(
        `${this.apiBaseUrl}/repos/${owner}/${repo}/branches?per_page=100`,
        { headers },
      )

      if (!response.ok) {
        throw new Error(`GitHub API error: ${response.status} ${response.statusText}`)
      }

      const branches = (await response.json()) as GitHubBranch[]
      return branches.map((b) => b.name)
    } catch (error) {
      logger.error({ err: error, owner, repo }, 'Error listing GitHub branches')
      throw error
    }
  }

  /**
   * Get the default branch name for a GitHub repository
   */
  async getDefaultBranch(remoteUrl: string, token?: string): Promise<string> {
    const { owner, repo } = this.parseUrl(remoteUrl)

    try {
      const headers: Record<string, string> = {
        Accept: 'application/vnd.github.v3+json',
      }

      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }

      const response = await fetch(`${this.apiBaseUrl}/repos/${owner}/${repo}`, { headers })

      if (!response.ok) {
        throw new Error(`GitHub API error: ${response.status} ${response.statusText}`)
      }

      const data = (await response.json()) as GitHubRepository
      return data.default_branch
    } catch (error) {
      logger.error({ err: error, owner, repo }, 'Error fetching default branch from GitHub')
      throw error
    }
  }

  /**
   * Parse GitHub repository URL to extract owner and repo name
   * Supports formats:
   * - https://github.com/owner/repo
   * - https://github.com/owner/repo.git
   */
  parseUrl(remoteUrl: string): { owner: string; repo: string } {
    const match = remoteUrl.match(/github\.com\/([^/]+)\/([^/.]+)/)
    if (!(match?.[1] && match[2])) {
      throw new Error(`Invalid GitHub URL: ${remoteUrl}`)
    }

    const owner = match[1]
    const repo = match[2].replace(/\.git$/, '')

    return { owner, repo }
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
      logger.error({ err: error as Error }, 'Error validating GitHub token')
      return false
    }
  }

  /**
   * Get the authenticated user's information
   */
  async getAuthenticatedUser(token: string): Promise<{ username: string }> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user`, {
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github.v3+json',
        },
      })

      if (!response.ok) {
        throw new Error(`GitHub API error: ${response.status} ${response.statusText}`)
      }

      const user = (await response.json()) as GitHubUser
      return { username: user.login }
    } catch (error) {
      logger.error({ err: error as Error }, 'Error fetching authenticated user from GitHub')
      throw error
    }
  }

  /**
   * Create a new repository on GitHub for the authenticated user
   */
  async createRepository(
    token: string,
    name: string,
    isPrivate: boolean,
  ): Promise<CreateRepositoryResult> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user/repos`, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github.v3+json',
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          name,
          private: isPrivate,
          auto_init: false, // Don't initialize with README since we're pushing existing code
        }),
      })

      if (!response.ok) {
        const errorBody = await response.text()
        logger.error(
          { status: response.status, statusText: response.statusText, errorBody },
          'GitHub API error creating repository',
        )

        if (response.status === 422) {
          throw new Error(`Repository '${name}' already exists or name is invalid`)
        }
        if (response.status === 401) {
          throw new Error('GitHub authentication failed - invalid or expired token')
        }
        if (response.status === 403) {
          throw new Error(
            'GitHub authorization failed - insufficient permissions to create repository',
          )
        }

        throw new Error(`GitHub API error: ${response.status} ${response.statusText}`)
      }

      const repo = (await response.json()) as GitHubRepository

      logger.info(
        { name: repo.name, fullName: repo.full_name, url: repo.html_url },
        'Successfully created GitHub repository',
      )

      return {
        url: repo.html_url,
        cloneUrl: repo.clone_url,
        defaultBranch: repo.default_branch,
        fullName: repo.full_name,
      }
    } catch (error) {
      logger.error({ err: error as Error, name }, 'Error creating GitHub repository')
      throw error
    }
  }

  /**
   * Get file content from a GitHub repository
   */
  async getFileContent(
    remoteUrl: string,
    path: string,
    token?: string,
    ref?: string,
  ): Promise<string | null> {
    const { owner, repo } = this.parseUrl(remoteUrl)

    try {
      const headers: Record<string, string> = {
        Accept: 'application/vnd.github.v3+json',
      }

      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }

      let url = `${this.apiBaseUrl}/repos/${owner}/${repo}/contents/${path}`
      if (ref) {
        url += `?ref=${encodeURIComponent(ref)}`
      }

      const response = await fetch(url, { headers })

      if (response.status === 404) {
        return null
      }

      if (!response.ok) {
        throw new Error(`GitHub API error: ${response.status} ${response.statusText}`)
      }

      const data = (await response.json()) as { content?: string; encoding?: string }

      if (data.encoding === 'base64' && data.content) {
        return Buffer.from(data.content, 'base64').toString('utf-8')
      }

      return null
    } catch (error) {
      logger.error({ err: error, owner, repo, path }, 'Error fetching file content from GitHub')
      return null
    }
  }
}

/**
 * Factory function to create GitHubProviderClient
 */
export function createGitHubProviderClient(): GitHubProviderClient {
  return new GitHubProviderClient()
}
