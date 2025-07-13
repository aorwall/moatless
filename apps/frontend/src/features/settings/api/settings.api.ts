import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/shared'
import { apiClient } from '@/lib/api/client'

export const settingsApi = {
  getSettings: async (): Promise<SettingsResponse> => {
    return apiClient.get('/api/v1/settings')
  },

  updateSettings: async (request: UpdateSettingsRequest): Promise<SettingsResponse> => {
    return apiClient.put('/api/v1/settings', request)
  },

  resetSettings: async (): Promise<SettingsResponse> => {
    return apiClient.delete('/api/v1/settings')
  },
}
