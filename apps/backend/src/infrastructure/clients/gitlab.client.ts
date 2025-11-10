import type { ServiceIdentifier } from 'inversify'
import { logger } from '../../utils/logger'

interface GitLabUser {
  id: number
  username: string
  email: string | null
  name: string | null
  avatar_url: string | null
}

interface GitLabApiUserResponse {
  id: number
  username: string
  email: string | null
  name: string | null
  avatar_url: string | null
}

/**
 * DI symbol for GitLabClient.
 * Use this symbol to inject GitLab client implementations.
 */
export const GitLabClientSymbol: ServiceIdentifier<GitLabClient> = Symbol.for('GitLabClient')

/**
 * Pure GitLab API client for making HTTP requests to GitLab's REST API v4
 * Contains no business logic - only API communication
 */
export class GitLabClient {
  private readonly apiBaseUrl = 'https://gitlab.com/api/v4'

  /**
   * Fetch the authenticated user's information
   */
  async getAuthenticatedUser(token: string): Promise<GitLabUser | null> {
    try {
      const response = await fetch(`${this.apiBaseUrl}/user`, {
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
      })

      if (!response.ok) {
        if (response.status === 401) {
          logger.error('GitLab API: Invalid or expired token')
        } else {
          logger.error('GitLab API: Failed to fetch user', new Error(`Status: ${response.status}`))
        }
        return null
      }

      const userData = (await response.json()) as GitLabApiUserResponse

      return {
        id: userData.id,
        username: userData.username,
        email: userData.email,
        name: userData.name,
        avatar_url: userData.avatar_url,
      }
    } catch (error) {
      logger.error('Error fetching GitLab user', error as Error)
      return null
    }
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
        },
      })

      return response.ok
    } catch (error) {
      logger.error('Error validating GitLab token', error as Error)
      return false
    }
  }

  /**
   * Get projects accessible by the authenticated user
   */
  async getUserProjects(
    token: string,
    options?: { page?: number; per_page?: number },
  ): Promise<
    | {
        id: number
        name: string
        path_with_namespace: string
        visibility: string
      }[]
    | null
  > {
    try {
      const page = options?.page || 1
      const perPage = options?.per_page || 30

      const response = await fetch(
        `${this.apiBaseUrl}/projects?page=${page}&per_page=${perPage}&owned=true&membership=true&order_by=updated_at`,
        {
          headers: {
            Authorization: `Bearer ${token}`,
            'Content-Type': 'application/json',
          },
        },
      )

      if (!response.ok) {
        logger.error(
          'GitLab API: Failed to fetch user projects',
          new Error(`Status: ${response.status}`),
        )
        return null
      }

      const projects = (await response.json()) as {
        id: number
        name: string
        path_with_namespace: string
        visibility: string
      }[]

      return projects
    } catch (error) {
      logger.error('Error fetching GitLab user projects', error as Error)
      return null
    }
  }

  /**
   * Get file content from a GitLab project
   * @param token GitLab personal access token
   * @param projectId Numeric project ID or URL-encoded path (e.g., "group%2Fproject")
   * @param path Path to the file in the repository
   * @param ref Optional branch/tag/commit ref (defaults to default branch)
   * @returns File content as string, or null if file doesn't exist or on error
   */
  async getFileContent(
    token: string,
    projectId: string | number,
    path: string,
    ref?: string,
  ): Promise<string | null> {
    try {
      const encodedPath = encodeURIComponent(path)
      const refParam = ref ? `?ref=${encodeURIComponent(ref)}` : ''
      const url = `${this.apiBaseUrl}/projects/${encodeURIComponent(projectId)}/repository/files/${encodedPath}/raw${refParam}`

      const response = await fetch(url, {
        headers: {
          Authorization: `Bearer ${token}`,
        },
      })

      if (!response.ok) {
        if (response.status === 404) {
          logger.debug('GitLab API: File not found', { projectId, path, ref })
          return null
        }
        logger.error(
          'GitLab API: Failed to fetch file content',
          new Error(`Status: ${response.status}`),
        )
        return null
      }

      // GitLab returns raw file content directly
      const content = await response.text()
      return content
    } catch (error) {
      logger.error('Error fetching file content from GitLab', error as Error)
      return null
    }
  }
}

/**
 * Factory function to create GitLabClient
 */
export function createGitLabClient(): GitLabClient {
  return new GitLabClient()
}
