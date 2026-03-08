import { apiClient } from '@/lib/api/client'

// ---- Types ----

export interface SlackChannelInfo {
  id: string
  name: string
}

export interface ChannelConfigResponse {
  id: string
  provider: string
  sourceId: string
  sourceName: string | null
  repositoryId: string
  routingMode: string
  promptTemplate: string | null
  active: boolean
  providerConfig: Record<string, unknown> | null
  ongoingTaskId: string | null
  createdBy: string
  createdAt: string
  updatedAt: string
}

export interface CreateChannelConfigRequest {
  provider: string
  sourceId: string
  sourceName?: string
  repositoryId: string
  routingMode: string
  promptTemplate?: string
  providerConfig?: Record<string, unknown>
}

export interface UpdateChannelConfigRequest {
  sourceName?: string
  routingMode?: string
  promptTemplate?: string | null
  active?: boolean
  providerConfig?: Record<string, unknown>
}

export interface PairingRequestResponse {
  id: number
  configId: string
  providerUserId: string
  code: string
  status: string
  meta: Record<string, unknown> | null
  createdAt: string
  updatedAt: string
}

// ---- API Client ----

const CONFIGS_PATH = '/api/v1/channels/configs'

export const channelsApi = {
  async listConfigs(params?: {
    provider?: string
    repositoryId?: string
  }): Promise<ChannelConfigResponse[]> {
    return await apiClient.get<ChannelConfigResponse[]>(CONFIGS_PATH, params)
  },

  async createConfig(request: CreateChannelConfigRequest): Promise<ChannelConfigResponse> {
    return await apiClient.post<ChannelConfigResponse>(CONFIGS_PATH, request)
  },

  async updateConfig(
    id: string,
    request: UpdateChannelConfigRequest,
  ): Promise<ChannelConfigResponse> {
    return await apiClient.put<ChannelConfigResponse>(`${CONFIGS_PATH}/${id}`, request)
  },

  async deleteConfig(id: string): Promise<void> {
    await apiClient.delete(`${CONFIGS_PATH}/${id}`)
  },

  async listSlackChannels(): Promise<SlackChannelInfo[]> {
    return await apiClient.get<SlackChannelInfo[]>('/api/v1/channels/slack/channels')
  },

  async listProviders(): Promise<string[]> {
    return await apiClient.get<string[]>('/api/v1/channels/providers')
  },

  async listMyPairingRequests(): Promise<PairingRequestResponse[]> {
    return await apiClient.get<PairingRequestResponse[]>('/api/v1/channels/pairing/my-requests')
  },

  async approvePairing(configId: string, code: string): Promise<void> {
    await apiClient.post('/api/v1/channels/telegram/pairing/approve', { configId, code })
  },
}
