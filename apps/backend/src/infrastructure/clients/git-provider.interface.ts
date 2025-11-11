/**
 * Common interface for Git provider clients (GitHub, GitLab)
 * Defines the contract for interacting with Git hosting providers via their APIs
 */
export interface GitProviderClient {
  /**
   * List all branches for a remote repository
   * @param remoteUrl - Full HTTPS URL to the repository
   * @param token - Optional authentication token
   * @returns Array of branch names
   */
  listBranches(remoteUrl: string, token?: string): Promise<string[]>

  /**
   * Get the default branch name for a repository
   * @param remoteUrl - Full HTTPS URL to the repository
   * @param token - Optional authentication token
   * @returns Default branch name (e.g., 'main', 'master')
   */
  getDefaultBranch(remoteUrl: string, token?: string): Promise<string>

  /**
   * Parse repository URL to extract owner and repo name
   * @param remoteUrl - Full HTTPS URL to the repository
   * @returns Object containing owner and repo name
   * @throws Error if URL format is invalid
   */
  parseUrl(remoteUrl: string): { owner: string; repo: string }

  /**
   * Validate a personal access token
   * @param token - Personal access token to validate
   * @returns True if token is valid, false otherwise
   */
  validateToken(token: string): Promise<boolean>
}
