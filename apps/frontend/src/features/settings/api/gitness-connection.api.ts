import type { GitnessConfigResponse, GitnessPATRequest } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

type DisconnectGitnessResponse = { success: boolean; message: string }
type SaveGitnessPATResponse = { success: boolean; message: string }

export const gitnessConnectionApi = {
  getConfig: async (): Promise<GitnessConfigResponse> => {
    const response = await apiClient.get<GitnessConfigResponse>('/api/v1/settings/gitness/config')
    return response
  },

  disconnect: async (): Promise<DisconnectGitnessResponse> => {
    const response = await apiClient.delete<DisconnectGitnessResponse>(
      '/api/v1/settings/gitness/config',
    )
    return response
  },

  savePAT: async (request: GitnessPATRequest): Promise<SaveGitnessPATResponse> => {
    const response = await apiClient.put<SaveGitnessPATResponse>('/api/v1/settings/gitness/pat', {
      baseUrl: request.baseUrl,
      token: request.accessToken,
    })
    return response
  },
}
