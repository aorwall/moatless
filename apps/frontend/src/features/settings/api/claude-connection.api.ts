import type {
  ClaudeConnectionConfig,
  ClaudeOAuthUrlResponse,
  ClaudePATRequest,
} from '@moatless/types'

type DisconnectClaudeResponse = { success: boolean; message: string }
type SaveClaudePATResponse = { success: boolean; message: string }

// TODO: Claude connection endpoints not implemented in Rust backend
export const claudeConnectionApi = {
  getOAuthUrl: (): Promise<ClaudeOAuthUrlResponse> => {
    return Promise.resolve({ authUrl: '', state: '' } as ClaudeOAuthUrlResponse)
  },

  getConfig: (): Promise<ClaudeConnectionConfig> => {
    return Promise.resolve({ connected: false, oauthEnabled: false } as ClaudeConnectionConfig)
  },

  disconnect: (): Promise<DisconnectClaudeResponse> => {
    return Promise.resolve({ success: false, message: 'Not implemented' })
  },

  savePAT: (_request: ClaudePATRequest): Promise<SaveClaudePATResponse> => {
    return Promise.resolve({ success: false, message: 'Not implemented' })
  },
}
