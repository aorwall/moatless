import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  deleteGithubConfig as deleteGithubConfigHandler,
  deleteGithubPatOverride,
  getGithubConfig as getGithubConfigHandler,
  saveGithubPat as saveGithubPatHandler,
} from '@moatless/api-client/sandbox/git-host-access/git-host-access'
import type {
  GitHubProviderTokenStatusResponse,
  SaveGitHubProviderTokenRequest,
} from '@moatless/api-client/sandbox/model'

export const githubConnectionApi = {
  /**
   * Get user's GitHub connection configuration
   */
  getConfig: async (): Promise<GitHubProviderTokenStatusResponse> => {
    return await unwrapResponse<GitHubProviderTokenStatusResponse>(getGithubConfigHandler())
  },

  /**
   * Disconnect GitHub service connection
   */
  disconnect: async (): Promise<GitHubProviderTokenStatusResponse> => {
    return await unwrapResponse<GitHubProviderTokenStatusResponse>(deleteGithubConfigHandler())
  },

  /**
   * Save Personal Access Token
   */
  savePAT: async (
    request: SaveGitHubProviderTokenRequest,
  ): Promise<GitHubProviderTokenStatusResponse> => {
    return await unwrapResponse<GitHubProviderTokenStatusResponse>(
      saveGithubPatHandler({ token: request.token }),
    )
  },

  /**
   * Remove a PAT override and restore the GitHub App user token.
   */
  removePATOverride: async (): Promise<GitHubProviderTokenStatusResponse> => {
    return await unwrapResponse<GitHubProviderTokenStatusResponse>(deleteGithubPatOverride())
  },
}
