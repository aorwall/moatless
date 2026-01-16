import type { ServiceIdentifier } from 'inversify'
import { logger } from '../../utils/logger'
import type { GitProviderClient } from './git-provider.interface'

interface GitLabBranch {
  name: string
  commit: {
    id: string
    short_id: string
  }
  protected: boolean
  default: boolean
}

interface GitLabProject {
  id: number
  name: string
  path: string
  path_with_namespace: string
  default_branch: string
  visibility: string
}

/**
 * DI symbol for GitLabProviderClient.
 * Use this symbol to inject GitLab provider client implementations.
 */
export const GitLabProviderClientSymbol: ServiceIdentifier<GitLabProviderClient> =
  Symbol.for('GitLabProviderClient')

/**
 * GitLab provider client implementing the GitProviderClient interface.
 * Provides repository metadata operations via GitLab REST API v4.
 * Supports nested groups (e.g., gitlab.com/group/subgroup/project)
 */
export class GitLabProviderClient implements GitProviderClient {
  private readonly apiBaseUrl = 'https://gitlab.com/api/v4'

  /**
   * List all branches for a GitLab project
   */
  async listBranches(remoteUrl: string, token?: string): Promise<string[]> {
    const projectPath = this.extractProjectPath(remoteUrl)

    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
      }

      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }

      // URL-encode the project path for the API call
      const encodedPath = encodeURIComponent(projectPath)
      const response = await fetch(
        `${this.apiBaseUrl}/projects/${encodedPath}/repository/branches?per_page=100`,
        { headers },
      )

      if (!response.ok) {
        throw new Error(`GitLab API error: ${response.status} ${response.statusText}`)
      }

      const branches = (await response.json()) as GitLabBranch[]
      return branches.map((b) => b.name)
    } catch (error) {
      logger.error({ err: error, projectPath }, 'Error listing GitLab branches')
      throw error
    }
  }

  /**
   * Get the default branch name for a GitLab project
   */
  async getDefaultBranch(remoteUrl: string, token?: string): Promise<string> {
    const projectPath = this.extractProjectPath(remoteUrl)

    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
      }

      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }

      // URL-encode the project path for the API call
      const encodedPath = encodeURIComponent(projectPath)
      const response = await fetch(`${this.apiBaseUrl}/projects/${encodedPath}`, { headers })

      if (!response.ok) {
        throw new Error(`GitLab API error: ${response.status} ${response.statusText}`)
      }

      const data = (await response.json()) as GitLabProject
      return data.default_branch
    } catch (error) {
      logger.error({ err: error, projectPath }, 'Error fetching default branch from GitLab')
      throw error
    }
  }

  /**
   * Parse GitLab repository URL to extract owner and repo name
   * Supports formats:
   * - https://gitlab.com/owner/repo
   * - https://gitlab.com/owner/repo.git
   * - https://gitlab.com/group/subgroup/project (nested groups)
   * - https://gitlab.com/group/subgroup/project.git
   *
   * Note: For nested groups, owner is the top-level group and repo is the project name
   */
  parseUrl(remoteUrl: string): { owner: string; repo: string } {
    const projectPath = this.extractProjectPath(remoteUrl)
    const parts = projectPath.split('/')

    if (parts.length < 2) {
      throw new Error(`Invalid GitLab URL: ${remoteUrl}`)
    }

    // For nested groups, owner is the top-level group, repo is the project name
    const owner = parts[0] || ''
    const repo = parts[parts.length - 1] || ''

    if (!(owner && repo)) {
      throw new Error(`Invalid GitLab URL: ${remoteUrl}`)
    }

    return { owner, repo }
  }

  /**
   * Validate a GitLab personal access token
   */
  async validateToken(token: string): Promise<boolean> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user`, {
        method: 'HEAD',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
      })

      return response.ok
    } catch (error) {
      logger.error({ err: error as Error }, 'Error validating GitLab token')
      return false
    }
  }

  /**
   * Get file content from a GitLab repository
   */
  async getFileContent(
    remoteUrl: string,
    path: string,
    token?: string,
    ref?: string,
  ): Promise<string | null> {
    const projectPath = this.extractProjectPath(remoteUrl)

    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
      }

      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }

      const encodedPath = encodeURIComponent(projectPath)
      const encodedFilePath = encodeURIComponent(path)
      let url = `${this.apiBaseUrl}/projects/${encodedPath}/repository/files/${encodedFilePath}/raw`
      if (ref) {
        url += `?ref=${encodeURIComponent(ref)}`
      }

      const response = await fetch(url, { headers })

      if (response.status === 404) {
        return null
      }

      if (!response.ok) {
        throw new Error(`GitLab API error: ${response.status} ${response.statusText}`)
      }

      return await response.text()
    } catch (error) {
      logger.error({ err: error, projectPath, path }, 'Error fetching file content from GitLab')
      return null
    }
  }

  /**
   * Extract the full project path from a GitLab URL
   * Supports nested groups (e.g., "group/subgroup/project")
   *
   * Examples:
   * - https://gitlab.com/owner/repo → "owner/repo"
   * - https://gitlab.com/group/subgroup/project.git → "group/subgroup/project"
   */
  private extractProjectPath(remoteUrl: string): string {
    const match = remoteUrl.match(/gitlab\.com\/(.+?)(?:\.git)?$/)
    if (!match?.[1]) {
      throw new Error(`Invalid GitLab URL: ${remoteUrl}`)
    }

    return match[1]
  }
}

/**
 * Factory function to create GitLabProviderClient
 */
export function createGitLabProviderClient(): GitLabProviderClient {
  return new GitLabProviderClient()
}
