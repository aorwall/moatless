import { apiClient } from '@/lib/api/client'

export interface CodexConfigResponse {
  connected: boolean
  tokenType?: string
  authMethod?: string
  email?: string
  planType?: string
  /** True when the stored credential needs a fresh sign-in (permanent refresh failure). */
  needsReconnect?: boolean
}

export interface CodexDeviceCodeStartResponse {
  deviceAuthId: string
  userCode: string
  verificationUrl: string
  intervalSecs: number
  expiresInSecs: number
}

export interface CodexDeviceCodePollRequest {
  deviceAuthId: string
  userCode: string
}

export type CodexDeviceCodePollResponse =
  | { status: 'pending' }
  | { status: 'success'; connected: boolean; email?: string; planType?: string }
  | { status: 'expired' }

export interface SaveCodexAuthRequest {
  authJson: string
}

export const codexConnectionApi = {
  getConfig: (): Promise<CodexConfigResponse> =>
    apiClient.get<CodexConfigResponse>('/api/v1/settings/codex/config'),

  saveAuthJson: (request: SaveCodexAuthRequest): Promise<CodexConfigResponse> =>
    apiClient.put<CodexConfigResponse>('/api/v1/settings/codex/config', request),

  disconnect: (): Promise<CodexConfigResponse> =>
    apiClient.delete<CodexConfigResponse>('/api/v1/settings/codex/config'),

  startDeviceLogin: (): Promise<CodexDeviceCodeStartResponse> =>
    apiClient.post<CodexDeviceCodeStartResponse>('/api/v1/settings/codex/login/start'),

  pollDeviceLogin: (request: CodexDeviceCodePollRequest): Promise<CodexDeviceCodePollResponse> =>
    apiClient.post<CodexDeviceCodePollResponse>('/api/v1/settings/codex/login/poll', request),
}
