import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  deleteGithubConfig as deleteGithubConfigHandler,
  exchangeGithubToken as exchangeGithubTokenHandler,
  getGithubConfig as getGithubConfigHandler,
  getGithubOauthUrl as getGithubOauthUrlHandler,
  saveGithubPat as saveGithubPatHandler,
} from '@moatless/api-client/sandbox/provider-settings/provider-settings'
import type {
  GitHubConfigResponse,
  GitHubOAuthTokenRequest,
  GitHubOAuthUrlResponse,
  GitHubPATRequest,
} from '@moatless/types'

type ExchangeGitHubTokenResponse = { success: boolean; message: string }
type DisconnectGitHubResponse = { success: boolean; message: string }
type SaveGitHubPATResponse = { success: boolean; message: string }

export const githubConnectionApi = {
  /**
   * Get GitHub OAuth authorization URL
   */
  getOAuthUrl: async (): Promise<GitHubOAuthUrlResponse> => {
    return await unwrapResponse<GitHubOAuthUrlResponse>(getGithubOauthUrlHandler())
  },

  /**
   * Exchange OAuth code for access token
   */
  exchangeToken: async (request: GitHubOAuthTokenRequest): Promise<ExchangeGitHubTokenResponse> => {
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
  savePAT: async (request: GitHubPATRequest): Promise<SaveGitHubPATResponse> => {
    return await unwrapResponse<SaveGitHubPATResponse>(
      saveGithubPatHandler({ token: request.accessToken }),
    )
  },
}
