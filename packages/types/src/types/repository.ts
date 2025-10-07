// Re-export types from schemas
export type {
  CodeExecution,
  CreateRepositoryRequest,
  PreviewConfig,
  RepositoryConfig,
  UpdateRepositoryRequest,
} from '../schemas/repository.schemas.js'

import type { PreviewConfig, RepositoryConfig as Repo } from '../schemas/repository.schemas.js'

export interface RepositoryVariable {
  key: string
  value: string
  description?: string
}

export interface RepositoryResponse extends Repo {
  // All fields are now inherited from RepositoryConfig (Repo)
}

export interface FileSearchRequest {
  repositoryId: string
  query: string
  limit?: number
}

export interface FileSearchResult {
  path: string
  absolutePath: string
  score: number
}

export interface FileSearchResponse {
  results: FileSearchResult[]
  totalFound: number
  query: string
}

// Also export with specific name to avoid conflicts
export type RepositoryFileSearchResponse = FileSearchResponse

export interface RepositoryList {
  repositories: RepositoryResponse[]
}

export interface SetupScriptCommand {
  command: string
  success: boolean
  exit_code: number
  output?: string
  error?: string
}

export interface SetupScriptResponse {
  success: boolean
  output?: string
  error?: string
  commands: SetupScriptCommand[]
}

export interface BranchListResponse {
  branches: string[]
  currentBranch?: string
}

export class RepositoryTemplate {
  id: string
  name: string
  gitUrl: string
  setupCommand?: string
  previewConfig?: PreviewConfig
  systemPrompt?: string

  constructor(
    id: string,
    name: string,
    gitUrl = 'https://github.com/aorwall/hello-world-ui.git',
    setupCommand?: string,
    previewConfig?: PreviewConfig,
    systemPrompt?: string,
  ) {
    this.id = id
    this.name = name
    this.gitUrl = gitUrl
    if (setupCommand !== undefined) {
      this.setupCommand = setupCommand
    }
    if (previewConfig !== undefined) {
      this.previewConfig = previewConfig
    }
    if (systemPrompt !== undefined) {
      this.systemPrompt = systemPrompt
    }
  }
}
