import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/shared'
import { apiClient } from '@/lib/api/client'

export const settingsApi = {
  getSettings: async (): Promise<SettingsResponse> => {
    return apiClient.get('/settings')
  },

  updateSettings: async (request: UpdateSettingsRequest): Promise<SettingsResponse> => {
    return apiClient.put('/settings', request)
  },

  resetSettings: async (): Promise<SettingsResponse> => {
    return apiClient.delete('/settings')
  },
}
