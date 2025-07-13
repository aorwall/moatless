import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export const settingsApi = {
  getSettings: async (): Promise<SettingsResponse> => {
    return apiClient.get<SettingsResponse>('/api/v1/settings')
  },

  updateSettings: async (request: UpdateSettingsRequest): Promise<SettingsResponse> => {
    return apiClient.put<SettingsResponse>('/api/v1/settings', request)
  },

  resetSettings: async (): Promise<SettingsResponse> => {
    return apiClient.delete<SettingsResponse>('/api/v1/settings')
  },
}
