import type {
  ClaudeSettingsResponse,
  GeneralSettingsResponse,
  GitHubSettingsResponse,
  SettingsResponse,
  SystemStatusCheck,
  UpdateClaudeSettingsRequest,
  UpdateGeneralSettingsRequest,
  UpdateGitHubSettingsRequest,
  UpdateSettingsRequest,
} from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export const settingsApi = {
  // Legacy monolithic endpoints
  getSettings: async (): Promise<SettingsResponse> => {
    return apiClient.get<SettingsResponse>('/api/v1/settings')
  },

  updateSettings: async (request: UpdateSettingsRequest): Promise<SettingsResponse> => {
    return apiClient.put<SettingsResponse>('/api/v1/settings', request)
  },

  resetSettings: async (): Promise<SettingsResponse> => {
    return apiClient.delete<SettingsResponse>('/api/v1/settings')
  },

  getSystemStatus: async (): Promise<SystemStatusCheck> => {
    return apiClient.get<SystemStatusCheck>('/api/v1/settings/status')
  },

  // Section-specific endpoints
  general: {
    get: async (): Promise<GeneralSettingsResponse> => {
      return apiClient.get<GeneralSettingsResponse>('/api/v1/settings/general')
    },
    update: async (request: UpdateGeneralSettingsRequest): Promise<GeneralSettingsResponse> => {
      return apiClient.put<GeneralSettingsResponse>('/api/v1/settings/general', request)
    },
  },

  claude: {
    get: async (): Promise<ClaudeSettingsResponse> => {
      return apiClient.get<ClaudeSettingsResponse>('/api/v1/settings/claude')
    },
    update: async (request: UpdateClaudeSettingsRequest): Promise<ClaudeSettingsResponse> => {
      return apiClient.put<ClaudeSettingsResponse>('/api/v1/settings/claude', request)
    },
  },

  github: {
    get: async (): Promise<GitHubSettingsResponse> => {
      return apiClient.get<GitHubSettingsResponse>('/api/v1/settings/github')
    },
    update: async (request: UpdateGitHubSettingsRequest): Promise<GitHubSettingsResponse> => {
      return apiClient.put<GitHubSettingsResponse>('/api/v1/settings/github', request)
    },
  },
}
