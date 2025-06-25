// Re-export types from schemas
export type {
  CodeExecution,
  CreateEnvironmentRequest,
  Environment,
  GithubRepository,
  LocalRepository,
  Repository,
  UpdateEnvironmentRequest,
} from '../schemas/environment.schemas.js'

import type {
  CodeExecution as CodeExec,
  GithubRepository as GithubRepo,
  Repository as Repo,
} from '../schemas/environment.schemas.js'

// Additional types not in schemas
export type RemoteRepository = GithubRepo

export interface EnvironmentVariable {
  key: string
  value: string
  description?: string
}

export interface EnvironmentResponse {
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
  environmentId: string
  query: string
  filePattern?: string
  maxResults?: number
}

export interface FileSearchResponse {
  files: string[]
  totalCount: number
  truncated: boolean
}

export interface EnvironmentList {
  environments: EnvironmentResponse[]
}

export interface SetupScriptResponse {
  success: boolean
  output?: string
  error?: string
}

export interface BranchListResponse {
  branches: string[]
  currentBranch?: string
}
