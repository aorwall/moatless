import type { GitLabConfigResponse } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { GitLabClient } from '../../../infrastructure/clients/gitlab.client'
import { GitLabClientSymbol } from '../../../infrastructure/clients/gitlab.client'
import { logger } from '../../../utils/logger'
import type { ProviderTokenRepository } from '../db/provider-token.repository'
import { SessionContext } from './session-context'

/**
 * GitLab Token Service
 *
 * Manages GitLab tokens for git operations with unified resolution strategy.
 * Supports PAT (Personal Access Token) authentication only.
 *
 * Token Resolution Priority:
 * 1. Session token (if user authenticated via GitLab OAuth with appropriate scope)
 * 2. Linked provider token (PAT stored in provider_tokens table)
 * 3. null
 *
 * Use Cases:
 * - User logs in via GitLab OAuth → session token automatically works for git ops
 * - User logs in via Auth0/other → can link GitLab via PAT
 */
@injectable()
export class GitLabTokenService {
  constructor(
    @inject(GitLabClientSymbol) private readonly client: GitLabClient,
    @inject('ProviderTokenRepository') private readonly tokenRepository: ProviderTokenRepository,
  ) {}

  /**
   * Get GitLab token for git operations with unified priority resolution.
   *
   * Priority:
   * 1. GitLab OAuth session token (if user logged in via GitLab)
   * 2. Linked provider token (PAT from provider_tokens table)
   * 3. null (no token available)
   *
   * @returns Token string or null if not available
   */
  async getTokenForGitOperations(): Promise<string | null> {
    const userId = SessionContext.getCurrentUserId()
    if (!userId) {
      logger.debug('No authenticated user for git operations')
      return null
    }

    // Priority 1: Check if user is authenticated via GitLab OAuth
    const session = SessionContext.getCurrentSession()
    if (session?.provider === 'gitlab' && session.accessToken) {
      logger.debug('Using GitLab OAuth session token for git operations', { userId })
      return session.accessToken
    }

    // Priority 2: Check for linked provider token (PAT)
    const providerToken = await this.tokenRepository.getProviderTokenRaw(userId, 'gitlab')
    if (providerToken) {
      logger.debug('Using linked GitLab provider token for git operations', {
        userId,
        method: providerToken.authMethod,
      })
      return providerToken.accessToken
    }

    logger.debug('No GitLab token available for git operations', { userId })
    return null
  }

  /**
   * Get GitLab connection configuration for current user.
   *
   * Returns connection status, method, and metadata about the GitLab connection.
   * Used by the settings UI to display current connection state.
   *
   * @returns Configuration with connection status and method
   */
  async getConnectionConfig(): Promise<GitLabConfigResponse> {
    const userId = SessionContext.getCurrentUserId()
    if (!userId) {
      return {
        connected: false,
      }
    }

    // Check GitLab OAuth session (primary authentication)
    const session = SessionContext.getCurrentSession()
    if (session?.provider === 'gitlab' && session.accessToken) {
      return {
        connected: true,
        method: 'gitlab-oauth',
        gitlabUsername: session.userLogin,
        gitlabUserId: session.userId,
      }
    }

    // Check linked provider token (PAT)
    const providerToken = await this.tokenRepository.getProviderTokenRaw(userId, 'gitlab')
    if (providerToken) {
      return {
        connected: true,
        method: 'pat',
        gitlabUsername: providerToken.providerLogin,
        gitlabUserId: providerToken.providerUserId,
        connectedAt: providerToken.connectedAt.toISOString(),
        expiresAt: providerToken.expiresAt?.toISOString(),
      }
    }

    return {
      connected: false,
    }
  }

  /**
   * Save Personal Access Token for GitLab service connection.
   *
   * Validates the token by fetching user info, then stores it in the database.
   *
   * @param userId - User identifier
   * @param accessToken - GitLab Personal Access Token
   * @throws Error if token is invalid
   */
  async saveProviderPAT(userId: string, accessToken: string): Promise<void> {
    // Validate token by fetching user info
    const user = await this.client.getAuthenticatedUser(accessToken)
    if (!user) {
      throw new Error('Invalid GitLab token')
    }

    await this.tokenRepository.saveProviderToken(userId, 'gitlab', {
      authMethod: 'pat',
      accessToken,
      connectedAt: new Date(),
      providerUserId: user.id.toString(),
      providerLogin: user.username,
    })

    logger.info('GitLab PAT saved successfully', {
      userId,
      gitlabUserId: user.id,
      gitlabUsername: user.username,
    })
  }

  /**
   * Delete user's GitLab provider token.
   *
   * Removes the linked PAT from the database.
   * Does not affect GitLab OAuth session (primary authentication).
   *
   * @param userId - User identifier
   */
  async deleteProviderToken(userId: string): Promise<void> {
    await this.tokenRepository.deleteProviderToken(userId, 'gitlab')
    logger.info('GitLab provider token deleted', { userId })
  }

  /**
   * Validate a GitLab token.
   * Delegates to GitLabClient.
   */
  async validateToken(token: string): Promise<boolean> {
    return await this.client.validateToken(token)
  }

  /**
   * Get projects accessible by the authenticated user.
   * Delegates to GitLabClient.
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
    return await this.client.getUserProjects(token, options)
  }

  /**
   * Get file content from a GitLab project.
   * Delegates to GitLabClient.
   */
  async getFileContent(
    token: string,
    projectId: string | number,
    path: string,
    ref?: string,
  ): Promise<string | null> {
    return await this.client.getFileContent(token, projectId, path, ref)
  }
}
