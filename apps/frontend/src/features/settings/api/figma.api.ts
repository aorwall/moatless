import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  deleteFigmaConfigRoute,
  exchangeFigmaTokenRoute,
  getFigmaConfigRoute,
  getFigmaOAuthUrlRoute,
} from '@moatless/api-contract/figma'
import { apiClient } from '@/lib/api/client'

type FigmaOAuthUrlResponse = RouteSuccess<typeof getFigmaOAuthUrlRoute>
type FigmaConfigResponse = RouteSuccess<typeof getFigmaConfigRoute>
type ExchangeFigmaTokenRequest = RouteRequestJson<typeof exchangeFigmaTokenRoute>
type ExchangeFigmaTokenResponse = RouteSuccess<typeof exchangeFigmaTokenRoute>
type DisconnectFigmaResponse = RouteSuccess<typeof deleteFigmaConfigRoute>

export const figmaApi = {
  /**
   * Get Figma OAuth authorization URL
   */
  getOAuthUrl: async (): Promise<FigmaOAuthUrlResponse> => {
    const path = buildPath(getFigmaOAuthUrlRoute)
    return await apiClient.get<FigmaOAuthUrlResponse>(path)
  },

  /**
   * Exchange OAuth code for access token
   */
  exchangeToken: async (
    request: ExchangeFigmaTokenRequest,
  ): Promise<ExchangeFigmaTokenResponse> => {
    const path = buildPath(exchangeFigmaTokenRoute)
    return await apiClient.post<ExchangeFigmaTokenResponse>(path, request)
  },

  /**
   * Get user's Figma configuration
   */
  getConfig: async (): Promise<FigmaConfigResponse> => {
    const path = buildPath(getFigmaConfigRoute)
    return await apiClient.get<FigmaConfigResponse>(path)
  },

  /**
   * Disconnect Figma OAuth
   */
  disconnect: async (): Promise<DisconnectFigmaResponse> => {
    const path = buildPath(deleteFigmaConfigRoute)
    return await apiClient.delete<DisconnectFigmaResponse>(path)
  },
}
