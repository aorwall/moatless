import type { ServerLogsResponse } from '@moatless/types'
import type { ServerStatus } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export const serverApi = {
  async startServer(taskId: string): Promise<ServerStatus> {
    return await apiClient.post<ServerStatus>(`/api/v1/tasks/${taskId}/server/start`)
  },

  async stopServer(taskId: string): Promise<ServerStatus> {
    return await apiClient.post<ServerStatus>(`/api/v1/tasks/${taskId}/server/stop`)
  },

  async restartServer(taskId: string): Promise<ServerStatus> {
    return await apiClient.post<ServerStatus>(`/api/v1/tasks/${taskId}/server/restart`)
  },

  async getServerStatus(taskId: string): Promise<ServerStatus> {
    return await apiClient.get<ServerStatus>(`/api/v1/tasks/${taskId}/server/status`)
  },

  async getServerLogs(taskId: string): Promise<ServerLogsResponse> {
    return await apiClient.get<ServerLogsResponse>(`/api/v1/tasks/${taskId}/server/logs`)
  },
}
