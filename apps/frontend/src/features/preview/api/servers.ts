import { apiClient } from '@/lib/api/client'

export interface ServerEntry {
  name: string
  label: string
  port: number
}

interface ServersResponse {
  servers: ServerEntry[]
}

export const serversApi = {
  async getServers(taskId: string): Promise<ServersResponse> {
    return await apiClient.get<ServersResponse>(`/api/v1/tasks/${taskId}/servers`)
  },
}
