import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  deleteClaudeConfigRoute,
  getClaudeConfigRoute,
  getClaudeOAuthUrlRoute,
  saveClaudePATRoute,
} from '@moatless/api-contract/settings'
import { apiClient } from '@/lib/api/client'

type ClaudeOAuthUrlResponse = RouteSuccess<typeof getClaudeOAuthUrlRoute>
type ClaudeConfigResponse = RouteSuccess<typeof getClaudeConfigRoute>
type DisconnectClaudeResponse = RouteSuccess<typeof deleteClaudeConfigRoute>
type SaveClaudePATRequest = RouteRequestJson<typeof saveClaudePATRoute>
type SaveClaudePATResponse = RouteSuccess<typeof saveClaudePATRoute>

export const claudeConnectionApi = {
  /**
   * Get Claude OAuth authorization URL
   */
  getOAuthUrl: async (): Promise<ClaudeOAuthUrlResponse> => {
    const path = buildPath(getClaudeOAuthUrlRoute)
    return await apiClient.get<ClaudeOAuthUrlResponse>(path)
  },

  /**
   * Get user's Claude connection configuration
   */
  getConfig: async (): Promise<ClaudeConfigResponse> => {
    const path = buildPath(getClaudeConfigRoute)
    return await apiClient.get<ClaudeConfigResponse>(path)
  },

  /**
   * Disconnect Claude service connection
   */
  disconnect: async (): Promise<DisconnectClaudeResponse> => {
    const path = buildPath(deleteClaudeConfigRoute)
    return await apiClient.delete<DisconnectClaudeResponse>(path)
  },

  /**
   * Save Personal Access Token
   */
  savePAT: async (request: SaveClaudePATRequest): Promise<SaveClaudePATResponse> => {
    const path = buildPath(saveClaudePATRoute)
    return await apiClient.put<SaveClaudePATResponse>(path, request)
  },
}
