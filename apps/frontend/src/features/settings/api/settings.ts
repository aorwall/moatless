import type { ClaudeSettingsResponse, UpdateClaudeSettingsRequest } from '@moatless/types'

// TODO: Claude settings endpoints not implemented in Rust backend
export const settingsApi = {
  claude: {
    get: (): Promise<ClaudeSettingsResponse> => {
      return Promise.resolve({} as ClaudeSettingsResponse)
    },
    update: (_request: UpdateClaudeSettingsRequest): Promise<ClaudeSettingsResponse> => {
      return Promise.resolve({} as ClaudeSettingsResponse)
    },
  },
}
