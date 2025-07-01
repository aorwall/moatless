// Re-export types from schemas
export type {
  CodeExecution,
  CreateRepositoryRequest,
  PreviewConfig,
  Repository,
  RepositoryConfig,
  ScriptInjectionStrategy,
  UpdateRepositoryRequest,
} from '../schemas/repository.schemas.js'

import type {
  CodeExecution as CodeExec,
  Repository as Repo,
} from '../schemas/repository.schemas.js'

export interface RepositoryVariable {
  key: string
  value: string
  description?: string
}

export interface RepositoryResponse {
  id: string
  name: string
  description?: string
  repository: Repo
  codeExecution: CodeExec
  variables: Record<string, string>
  createdAt: Date
  updatedAt: Date
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
