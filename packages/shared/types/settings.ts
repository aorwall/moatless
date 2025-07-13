export interface ApplicationSettings {
  dataDir?: string
  defaultWorkspaceType?: 'docker' | 'local'
  claude?: ClaudeSettings
}

export interface ClaudeSettings {
  binDir?: string
  configDir?: string
}

export interface UpdateSettingsRequest {
  dataDir?: string | null
  defaultWorkspaceType?: 'docker' | 'local' | null
  claude?: {
    binDir?: string | null
    configDir?: string | null
  }
}

export interface SettingsResponse {
  settings: ApplicationSettings
  defaults: {
    dataDir: string
    defaultWorkspaceType: 'docker' | 'local'
    claude: {
      binDir: string
      configDir: string
    }
  }
}
