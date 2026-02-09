import type {
  FigmaOAuthTokenRequest,
  FigmaOAuthUrlResponse,
  FigmaPATRequest,
} from '@moatless/types'

interface FigmaConfigResponse {
  connected: boolean
  oauthEnabled: boolean
  authMethod?: 'oauth' | 'pat'
  figmaUserId?: string
  connectedAt?: string
  expiresAt?: string
}

type ExchangeFigmaTokenResponse = { success: boolean; message: string }
type DisconnectFigmaResponse = { success: boolean; message: string }
type SaveFigmaPATResponse = { success: boolean; message: string }

// TODO: Figma config endpoints not implemented in Rust backend
export const figmaApi = {
  getOAuthUrl: (): Promise<FigmaOAuthUrlResponse> => {
    return Promise.resolve({ authUrl: '', state: '' } as FigmaOAuthUrlResponse)
  },

  exchangeToken: (_request: FigmaOAuthTokenRequest): Promise<ExchangeFigmaTokenResponse> => {
    return Promise.resolve({ success: false, message: 'Not implemented' })
  },

  getConfig: (): Promise<FigmaConfigResponse> => {
    return Promise.resolve({ connected: false, oauthEnabled: false })
  },

  disconnect: (): Promise<DisconnectFigmaResponse> => {
    return Promise.resolve({ success: false, message: 'Not implemented' })
  },

  savePAT: (_request: FigmaPATRequest): Promise<SaveFigmaPATResponse> => {
    return Promise.resolve({ success: false, message: 'Not implemented' })
  },
}
