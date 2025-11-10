import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  deleteGitLabConfigRoute,
  getGitLabConfigRoute,
  saveGitLabPATRoute,
} from '@moatless/api-contract/settings'
import { apiClient } from '@/lib/api/client'

type GitLabConfigResponse = RouteSuccess<typeof getGitLabConfigRoute>
type DisconnectGitLabResponse = RouteSuccess<typeof deleteGitLabConfigRoute>
type SaveGitLabPATRequest = RouteRequestJson<typeof saveGitLabPATRoute>
type SaveGitLabPATResponse = RouteSuccess<typeof saveGitLabPATRoute>

export const gitlabConnectionApi = {
  /**
   * Get user's GitLab connection configuration
   */
  getConfig: async (): Promise<GitLabConfigResponse> => {
    const path = buildPath(getGitLabConfigRoute)
    return await apiClient.get<GitLabConfigResponse>(path)
  },

  /**
   * Disconnect GitLab service connection
   */
  disconnect: async (): Promise<DisconnectGitLabResponse> => {
    const path = buildPath(deleteGitLabConfigRoute)
    return await apiClient.delete<DisconnectGitLabResponse>(path)
  },

  /**
   * Save Personal Access Token
   */
  savePAT: async (request: SaveGitLabPATRequest): Promise<SaveGitLabPATResponse> => {
    const path = buildPath(saveGitLabPATRoute)
    return await apiClient.put<SaveGitLabPATResponse>(path, request)
  },
}
