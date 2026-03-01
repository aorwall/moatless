/**
 * Settings provider types for endpoints not yet implemented in the Rust backend.
 * When the backend adds these endpoints, replace with api-client generated types.
 */

// GitLab (TODO: not implemented in backend)
export interface GitLabConfigResponse {
  connected: boolean
  method?: 'gitlab-oauth' | 'pat'
  gitlabUserId?: string
  gitlabUsername?: string
  connectedAt?: string
  expiresAt?: string
}

export interface GitLabPATRequest {
  accessToken: string
}

// Claude settings (TODO: not implemented in backend)
export interface ClaudeSettingsResponse {
  oauthToken: string
}

export interface UpdateClaudeSettingsRequest {
  oauthToken?: string | null
}

export interface ClaudeConnectionConfig {
  connected: boolean
  oauthEnabled: boolean
  method?: 'oauth' | 'pat' | 'file' | 'api_key'
  claudeUserId?: string
  connectedAt?: string
}

export interface ClaudeOAuthUrlResponse {
  authUrl: string
  state: string
}

export interface ClaudePATRequest {
  accessToken: string
}

// Figma settings (TODO: not implemented in backend)
export interface FigmaOAuthTokenRequest {
  code: string
  state: string
}

export interface FigmaOAuthUrlResponse {
  authUrl: string
  state: string
}

export interface FigmaPATRequest {
  accessToken: string
}
