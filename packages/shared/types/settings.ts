export interface ApplicationSettings {
  dataDir?: string
  defaultWorkspaceType?: 'docker' | 'local'
  claude?: ClaudeSettings
}

export interface ClaudeSettings {
  binDir?: string
  configDir?: string
}

// Import UpdateSettingsRequest from schema
export type { UpdateSettingsSchemaType as UpdateSettingsRequest } from '../schemas/settings.schemas.js'

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
