import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  getClaudeSettingsRoute,
  getGeneralSettingsRoute,
  getMcpSettingsRoute,
  getPublishSettingsRoute,
  updateClaudeSettingsRoute,
  updateGeneralSettingsRoute,
  updateMcpSettingsRoute,
  updatePublishSettingsRoute,
} from '@moatless/api-contract/settings.ts'
import { apiClient } from '@/lib/api/client'

type GeneralSettingsResponse = RouteSuccess<typeof getGeneralSettingsRoute>
type ClaudeSettingsResponse = RouteSuccess<typeof getClaudeSettingsRoute>
type McpSettingsResponse = RouteSuccess<typeof getMcpSettingsRoute>
type PublishSettingsResponse = RouteSuccess<typeof getPublishSettingsRoute>
type UpdateGeneralSettingsRequest = RouteRequestJson<typeof updateGeneralSettingsRoute>
type UpdateClaudeSettingsRequest = RouteRequestJson<typeof updateClaudeSettingsRoute>
type UpdateMcpSettingsRequest = RouteRequestJson<typeof updateMcpSettingsRoute>
type UpdatePublishSettingsRequest = RouteRequestJson<typeof updatePublishSettingsRoute>

export const settingsApi = {
  // Section-specific endpoints
  general: {
    get: async (): Promise<GeneralSettingsResponse> => {
      const path = buildPath(getGeneralSettingsRoute)
      return await apiClient.get<GeneralSettingsResponse>(path)
    },
    update: async (request: UpdateGeneralSettingsRequest): Promise<GeneralSettingsResponse> => {
      const path = buildPath(updateGeneralSettingsRoute)
      return await apiClient.put<GeneralSettingsResponse>(path, request)
    },
  },

  claude: {
    get: async (): Promise<ClaudeSettingsResponse> => {
      const path = buildPath(getClaudeSettingsRoute)
      return await apiClient.get<ClaudeSettingsResponse>(path)
    },
    update: async (request: UpdateClaudeSettingsRequest): Promise<ClaudeSettingsResponse> => {
      const path = buildPath(updateClaudeSettingsRoute)
      return await apiClient.put<ClaudeSettingsResponse>(path, request)
    },
  },

  mcp: {
    get: async (): Promise<McpSettingsResponse> => {
      const path = buildPath(getMcpSettingsRoute)
      return await apiClient.get<McpSettingsResponse>(path)
    },
    update: async (request: UpdateMcpSettingsRequest): Promise<McpSettingsResponse> => {
      const path = buildPath(updateMcpSettingsRoute)
      return await apiClient.put<McpSettingsResponse>(path, request)
    },
  },

  publish: {
    get: async (): Promise<PublishSettingsResponse> => {
      const path = buildPath(getPublishSettingsRoute)
      return await apiClient.get<PublishSettingsResponse>(path)
    },
    update: async (request: UpdatePublishSettingsRequest): Promise<PublishSettingsResponse> => {
      const path = buildPath(updatePublishSettingsRoute)
      return await apiClient.put<PublishSettingsResponse>(path, request)
    },
  },
}
