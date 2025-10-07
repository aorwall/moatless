import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  getClaudeSettingsRoute,
  getGeneralSettingsRoute,
  getGitHubSettingsRoute,
  getPublishSettingsRoute,
  getSystemStatusRoute,
  updateClaudeSettingsRoute,
  updateGeneralSettingsRoute,
  updateGitHubSettingsRoute,
  updatePublishSettingsRoute,
} from '@moatless/api-contract/settings.ts'
import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

type SystemStatusCheck = RouteSuccess<typeof getSystemStatusRoute>
type GeneralSettingsResponse = RouteSuccess<typeof getGeneralSettingsRoute>
type ClaudeSettingsResponse = RouteSuccess<typeof getClaudeSettingsRoute>
type GitHubSettingsResponse = RouteSuccess<typeof getGitHubSettingsRoute>
type PublishSettingsResponse = RouteSuccess<typeof getPublishSettingsRoute>
type UpdateGeneralSettingsRequest = RouteRequestJson<typeof updateGeneralSettingsRoute>
type UpdateClaudeSettingsRequest = RouteRequestJson<typeof updateClaudeSettingsRoute>
type UpdateGitHubSettingsRequest = RouteRequestJson<typeof updateGitHubSettingsRoute>
type UpdatePublishSettingsRequest = RouteRequestJson<typeof updatePublishSettingsRoute>

export const settingsApi = {
  // Legacy monolithic endpoints - TODO: Migrate to section-specific endpoints or add route descriptors
  getSettings: async (): Promise<SettingsResponse> => {
    return await apiClient.get<SettingsResponse>('/api/v1/settings')
  },

  updateSettings: async (request: UpdateSettingsRequest): Promise<SettingsResponse> => {
    return await apiClient.put<SettingsResponse>('/api/v1/settings', request)
  },

  resetSettings: async (): Promise<SettingsResponse> => {
    return await apiClient.delete<SettingsResponse>('/api/v1/settings')
  },

  getSystemStatus: async (): Promise<SystemStatusCheck> => {
    const path = buildPath(getSystemStatusRoute)
    return await apiClient.get<SystemStatusCheck>(path)
  },

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

  github: {
    get: async (): Promise<GitHubSettingsResponse> => {
      const path = buildPath(getGitHubSettingsRoute)
      return await apiClient.get<GitHubSettingsResponse>(path)
    },
    update: async (request: UpdateGitHubSettingsRequest): Promise<GitHubSettingsResponse> => {
      const path = buildPath(updateGitHubSettingsRoute)
      return await apiClient.put<GitHubSettingsResponse>(path, request)
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
