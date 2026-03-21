import { customInstance, unwrapResponse } from '@moatless/api-client/custom-instance'
import type {
  ExchangeTokenRequest,
  GitHubConfigResponse,
  OAuthUrlResponse,
  SavePatRequest,
} from '@moatless/api-client/sandbox/model'
import {
  deleteGithubConfig as deleteGithubConfigHandler,
  exchangeGithubToken as exchangeGithubTokenHandler,
  getGithubConfig as getGithubConfigHandler,
  getGithubOauthUrl as getGithubOauthUrlHandler,
  saveGithubPat as saveGithubPatHandler,
} from '@moatless/api-client/sandbox/provider-settings/provider-settings'

type ExchangeGitHubTokenResponse = { success: boolean; message: string }
type DisconnectGitHubResponse = { success: boolean; message: string }
type SaveGitHubPATResponse = { success: boolean; message: string }
type RemovePATOverrideResponse = { data: GitHubConfigResponse; status: number; headers: Headers }

export const githubConnectionApi = {
  /**
   * Get GitHub OAuth authorization URL
   */
  getOAuthUrl: async (): Promise<OAuthUrlResponse> => {
    return await unwrapResponse<OAuthUrlResponse>(getGithubOauthUrlHandler())
  },

  /**
   * Exchange OAuth code for access token
   */
  exchangeToken: async (request: ExchangeTokenRequest): Promise<ExchangeGitHubTokenResponse> => {
    return await unwrapResponse<ExchangeGitHubTokenResponse>(exchangeGithubTokenHandler(request))
  },

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
   * Remove PAT override (restore OAuth token for GitHub-OAuth users)
   */
  removePATOverride: async (): Promise<GitHubConfigResponse> => {
    return await unwrapResponse<GitHubConfigResponse>(
      customInstance<RemovePATOverrideResponse>('/api/v1/settings/github/pat', {
        method: 'DELETE',
      }),
    )
  },
}
