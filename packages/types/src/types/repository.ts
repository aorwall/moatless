import type { RepositoryConfig as Repo } from '../schemas/repository.schemas.js'

export interface RepositoryVariable {
  key: string
  value: string
  description?: string
}

// RepositoryResponse is the same as RepositoryConfig (which is a discriminated union)
export type RepositoryResponse = Repo

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
