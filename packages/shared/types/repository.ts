// Re-export types from schemas
export type {
  CodeExecution,
  CreateRepositoryRequest,
  PreviewConfig,
  RepositoryConfig,
  ScriptInjectionStrategy,
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

export interface RepositoryFileSearchResponse {
  results: FileSearchResult[]
  totalFound: number
  query: string
}

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

  constructor(
    id: string,
    name: string,
    gitUrl: string = 'https://github.com/aorwall/hello-world-ui.git',
    setupCommand?: string,
    previewConfig?: PreviewConfig,
  ) {
    this.id = id
    this.name = name
    this.gitUrl = gitUrl
    this.setupCommand = setupCommand
    this.previewConfig = previewConfig
  }
}
