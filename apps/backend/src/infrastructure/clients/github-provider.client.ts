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
      logger.error('Error listing GitHub branches', { owner, repo, error })
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
      logger.error('Error fetching default branch from GitHub', { owner, repo, error })
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
      logger.error('Error validating GitHub token', error as Error)
      return false
    }
  }
}

/**
 * Factory function to create GitHubProviderClient
 */
export function createGitHubProviderClient(): GitHubProviderClient {
  return new GitHubProviderClient()
}
