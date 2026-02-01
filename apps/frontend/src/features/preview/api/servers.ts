import { apiClient } from '@/lib/api/client'

export interface ServerCapabilities {
  addressBar: boolean
  inspector: boolean
  reload: boolean
  openExternal: boolean
  viewport: boolean
  console: boolean
}

export interface ServerEntry {
  name: string
  label: string
  port: number
  status: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
  url?: string
  error?: string
  default?: boolean
  capabilities: ServerCapabilities
}

interface ServersResponse {
  servers: ServerEntry[]
}

export const serversApi = {
  async getServers(taskId: string): Promise<ServersResponse> {
    return await apiClient.get<ServersResponse>(`/api/v1/tasks/${taskId}/servers`)
  },
}
