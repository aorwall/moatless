import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { GitHubTokenService } from './github-token.service'
import { GitLabTokenService } from './gitlab-token.service'

/**
 * GitProviderTokenService - Unified token retrieval for all Git providers
 *
 * Provides centralized token management across GitHub, GitLab, and other Git providers.
 * Automatically detects the provider from repository URLs and retrieves appropriate tokens.
 *
 * Benefits:
 * - Single dependency injection point per service
 * - Centralized provider detection logic
 * - Easy to extend with new providers
 * - Consistent token retrieval interface
 *
 * @example
 * ```typescript
 * // Auto-detect provider from URL
 * const token = await gitProviderTokenService.getTokenForGitOperations(
 *   'https://github.com/user/repo',
 *   sessionToken
 * )
 *
 * // Or specify provider directly
 * const provider = gitProviderTokenService.detectProvider(remoteUrl)
 * const token = await gitProviderTokenService.getTokenForProvider(provider, sessionToken)
 * ```
 */
@injectable()
export class GitProviderTokenService {
  constructor(
    @inject(GitHubTokenService) private readonly githubTokenService: GitHubTokenService,
    @inject(GitLabTokenService) private readonly gitlabTokenService: GitLabTokenService,
  ) {}

  /**
   * Get access token for git operations based on remote URL
   *
   * Auto-detects the provider from the URL and retrieves the appropriate token.
   * Supports GitHub OAuth session tokens and provider-specific PATs.
   *
   * @param remoteUrl - Git repository URL (e.g., https://github.com/user/repo)
   * @param sessionToken - Optional session token override (for GitHub OAuth)
   * @returns Token string or null if not available
   */
  getTokenForGitOperations(remoteUrl: string, sessionToken?: string): Promise<string | null> {
    const provider = this.detectProvider(remoteUrl)
    return this.getTokenForProvider(provider, sessionToken)
  }

  /**
   * Detect git provider from repository URL
   *
   * Examines the URL to determine which Git hosting provider it belongs to.
   * Currently supports GitHub and GitLab only.
   *
   * @param remoteUrl - Git repository URL
   * @returns Provider identifier ('github' | 'gitlab')
   * @throws Error if provider is not supported
   */
  detectProvider(remoteUrl: string): 'github' | 'gitlab' {
    if (remoteUrl.includes('github.com')) {
      return 'github'
    }
    if (remoteUrl.includes('gitlab.com')) {
      return 'gitlab'
    }
    throw new Error(
      `Unsupported Git provider. Only GitHub and GitLab are supported. URL: ${remoteUrl}`,
    )
  }

  /**
   * Get token for specific provider (when provider already known)
   *
   * Useful when the provider has already been detected and you want to avoid
   * redundant provider detection.
   *
   * @param provider - Provider identifier ('github' | 'gitlab')
   * @param sessionToken - Optional session token override (for GitHub OAuth)
   * @returns Token string or null if not available
   */
  async getTokenForProvider(
    provider: 'github' | 'gitlab',
    sessionToken?: string,
  ): Promise<string | null> {
    // Guard against empty string tokens - treat them as missing tokens
    const validSessionToken = sessionToken?.trim() ? sessionToken : null

    if (provider === 'github') {
      // GitHub supports both OAuth session tokens and PATs
      // Session token takes precedence if provided
      return validSessionToken || (await this.githubTokenService.getTokenForGitOperations())
    }

    if (provider === 'gitlab') {
      // GitLab only supports PATs (no OAuth yet)
      return await this.gitlabTokenService.getTokenForGitOperations()
    }

    // TypeScript exhaustiveness check - should never reach here
    const _exhaustiveCheck: never = provider
    throw new Error(`Unsupported provider: ${_exhaustiveCheck}`)
  }

  /**
   * Check if the current user has a GitHub token available for git operations.
   * Used for early validation before creating tasks that require git access.
   *
   * This method checks the same sources as getTokenForGitOperations but returns
   * a boolean without actually retrieving the token.
   *
   * @returns true if a GitHub token is available, false otherwise
   */
  async hasGitHubTokenForCurrentUser(): Promise<boolean> {
    const token = await this.githubTokenService.getTokenForGitOperations()
    const hasToken = token !== null && token !== 'placeholder'

    logger.debug({ hasToken }, 'Checked GitHub token availability for current user')
    return hasToken
  }

  /**
   * Build authenticated clone URL for any Git provider
   *
   * Inserts the access token into the HTTPS URL for authenticated cloning.
   * Supports GitHub and GitLab.
   *
   * @param remoteUrl - Original repository URL
   * @param token - Access token (PAT or OAuth token)
   * @param provider - Provider identifier ('github' | 'gitlab')
   * @returns Authenticated clone URL or original URL if no token
   */
  buildAuthenticatedCloneUrl(
    remoteUrl: string,
    token: string | null,
    provider: 'github' | 'gitlab',
  ): string {
    if (!token) {
      return remoteUrl
    }

    if (provider === 'github') {
      return remoteUrl.replace(/^https:\/\/github\.com/, `https://${token}@github.com`)
    }

    if (provider === 'gitlab') {
      // GitLab requires username:token format (username can be any non-empty string)
      // Using 'oauth2' prefix is a common convention for token-based authentication
      return remoteUrl.replace(/^https:\/\/gitlab\.com/, `https://oauth2:${token}@gitlab.com`)
    }

    // TypeScript exhaustiveness check - should never reach here
    throw new Error(`Unsupported provider: ${provider}`)
  }
}
