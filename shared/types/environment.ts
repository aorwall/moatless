export interface Environment {
  id: string
  name: string
  description?: string
  repository: Repository
  codeExecution: CodeExecution
  variables: Record<string, string>
  createdAt: Date
  updatedAt: Date
  isActive: boolean
}

export interface Repository {
  type: 'local' | 'github' | 'gitlab'
  path?: string
  url?: string
  branch?: string
  commit?: string
}

export interface LocalRepository extends Repository {
  type: 'local'
  path: string
  useGitWorktrees?: boolean
  originalBranch?: string
}

export interface GithubRepository extends Repository {
  type: 'github'
  repoId: string
  branch: string
  commit?: string
  token?: string
}

export type RemoteRepository = GithubRepository

export interface CodeExecution {
  type: 'local' | 'docker' | 'sandbox'
  setupCommands?: string
  serverCommand?: string
  serverPortPlaceholder?: string
  serverLogFile?: string
  image?: string
  envVars?: Record<string, string>
  volumes?: string[]
  ports?: Record<string, number>
}

export interface EnvironmentVariable {
  key: string
  value: string
  description?: string
}

export interface CreateEnvironmentRequest {
  name: string
  description?: string
  repository: Repository
  codeExecution: CodeExecution
  variables?: Record<string, string>
}

export interface UpdateEnvironmentRequest {
  name?: string
  description?: string
  repository?: Repository
  codeExecution?: CodeExecution
  variables?: Record<string, string>
  isActive?: boolean
}

export interface EnvironmentResponse extends Environment {
  id: string
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
