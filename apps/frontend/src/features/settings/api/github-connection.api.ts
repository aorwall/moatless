import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  deleteGitHubConfigRoute,
  exchangeGitHubTokenRoute,
  getGitHubConfigRoute,
  getGitHubOAuthUrlRoute,
  saveGitHubPATRoute,
} from '@moatless/api-contract/settings'
import { apiClient } from '@/lib/api/client'

type GitHubOAuthUrlResponse = RouteSuccess<typeof getGitHubOAuthUrlRoute>
type GitHubConfigResponse = RouteSuccess<typeof getGitHubConfigRoute>
type ExchangeGitHubTokenRequest = RouteRequestJson<typeof exchangeGitHubTokenRoute>
type ExchangeGitHubTokenResponse = RouteSuccess<typeof exchangeGitHubTokenRoute>
type DisconnectGitHubResponse = RouteSuccess<typeof deleteGitHubConfigRoute>
type SaveGitHubPATRequest = RouteRequestJson<typeof saveGitHubPATRoute>
type SaveGitHubPATResponse = RouteSuccess<typeof saveGitHubPATRoute>

export const githubConnectionApi = {
  /**
   * Get GitHub OAuth authorization URL
   */
  getOAuthUrl: async (): Promise<GitHubOAuthUrlResponse> => {
    const path = buildPath(getGitHubOAuthUrlRoute)
    return await apiClient.get<GitHubOAuthUrlResponse>(path)
  },

  /**
   * Exchange OAuth code for access token
   */
  exchangeToken: async (
    request: ExchangeGitHubTokenRequest,
  ): Promise<ExchangeGitHubTokenResponse> => {
    const path = buildPath(exchangeGitHubTokenRoute)
    return await apiClient.post<ExchangeGitHubTokenResponse>(path, request)
  },

  /**
   * Get user's GitHub connection configuration
   */
  getConfig: async (): Promise<GitHubConfigResponse> => {
    const path = buildPath(getGitHubConfigRoute)
    return await apiClient.get<GitHubConfigResponse>(path)
  },

  /**
   * Disconnect GitHub service connection
   */
  disconnect: async (): Promise<DisconnectGitHubResponse> => {
    const path = buildPath(deleteGitHubConfigRoute)
    return await apiClient.delete<DisconnectGitHubResponse>(path)
  },

  /**
   * Save Personal Access Token
   */
  savePAT: async (request: SaveGitHubPATRequest): Promise<SaveGitHubPATResponse> => {
    const path = buildPath(saveGitHubPATRoute)
    return await apiClient.put<SaveGitHubPATResponse>(path, request)
  },
}
