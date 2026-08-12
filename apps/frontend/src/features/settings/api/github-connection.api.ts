import { customInstance, unwrapResponse } from '@moatless/api-client/custom-instance'
import type { GitHubConfigResponse, SavePatRequest } from '@moatless/api-client/sandbox/model'
import {
  deleteGithubConfig as deleteGithubConfigHandler,
  getGithubConfig as getGithubConfigHandler,
  saveGithubPat as saveGithubPatHandler,
} from '@moatless/api-client/sandbox/provider-settings/provider-settings'

type DisconnectGitHubResponse = { success: boolean; message: string }
type SaveGitHubPATResponse = { success: boolean; message: string }
type RemovePATOverrideResponse = { data: GitHubConfigResponse; status: number; headers: Headers }

export const githubConnectionApi = {
  /**
   * Get user's GitHub connection configuration
   */
  getConfig: async (): Promise<GitHubConfigResponse> => {
    return await unwrapResponse<GitHubConfigResponse>(getGithubConfigHandler())
  },

  /**
   * Disconnect GitHub service connection
   */
  disconnect: async (): Promise<DisconnectGitHubResponse> => {
    return await unwrapResponse<DisconnectGitHubResponse>(deleteGithubConfigHandler())
  },

  /**
   * Save Personal Access Token
   */
  savePAT: async (request: SavePatRequest): Promise<SaveGitHubPATResponse> => {
    return await unwrapResponse<SaveGitHubPATResponse>(
      saveGithubPatHandler({ token: request.token }),
    )
  },

  /**
   * Remove a PAT override and restore the GitHub App user token.
   */
  removePATOverride: async (): Promise<GitHubConfigResponse> => {
    return await unwrapResponse<GitHubConfigResponse>(
      customInstance<RemovePATOverrideResponse>('/api/v1/settings/github/pat', {
        method: 'DELETE',
      }),
    )
  },
}
