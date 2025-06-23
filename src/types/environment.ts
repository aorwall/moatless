export interface EnvironmentVariable {
  key: string;
  value: string;
  description?: string;
}

// Repository types
export interface LocalRepository {
  type: 'local';
  path: string;
  use_git_worktrees?: boolean;
}

export interface GithubRepository {
  type: 'github';
  repo_id: string; // Format: "owner/repo"
  branch?: string;
  commit?: string;
  token?: string; // For private repos
}

export type Repository = LocalRepository | GithubRepository;

// Code execution types
export interface CodeExecution {
  type: 'local' | 'docker';
  setup_commands?: string;
  server_command?: string;
  // Docker-specific fields
  image?: string; // Required when type="docker"
  env_vars?: Record<string, string>;
  volumes?: Array<string>;
  ports?: Record<string, number>;
}

export interface Environment {
  id: string;
  name: string;
  description?: string;
  repository: Repository;
  code_execution: CodeExecution;
  variables: Record<string, string>;
  created_at: string;
  updated_at: string;
  is_active: boolean;
}

export interface CreateEnvironmentRequest {
  name: string;
  description?: string;
  repository: Repository;
  code_execution: CodeExecution;
  variables?: Record<string, string>;
}

export interface UpdateEnvironmentRequest {
  name?: string;
  description?: string;
  repository?: Repository;
  code_execution?: CodeExecution;
  variables?: Record<string, string>;
  is_active?: boolean;
}

export interface EnvironmentResponse {
  id: string;
  name: string;
  description?: string;
  repository: Repository;
  code_execution: CodeExecution;
  variables: Record<string, string>;
  created_at: string;
  updated_at: string;
  is_active: boolean;
}

export interface EnvironmentList {
  environments: Array<EnvironmentResponse>;
  total: number;
}

// Setup script execution types
export interface SetupScriptCommand {
  command: string;
  output: string;
  error: string;
  exit_code: number;
  success: boolean;
}

export interface SetupScriptResponse {
  environment_id: string;
  session_id: string;
  working_directory: string;
  commands: Array<SetupScriptCommand>;
  overall_success: boolean;
  total_duration_ms: number;
}

// File search types
export interface FileSearchRequest {
  query: string;
  limit?: number;
}

export interface FileSearchResult {
  path: string;
  absolute_path: string;
  score: number;
}

export interface FileSearchResponse {
  results: Array<FileSearchResult>;
  total_found: number;
  query: string;
}

// Branch types
export interface Branch {
  name: string;
  is_current: boolean;
  last_commit_date: string;
}

export interface BranchListResponse {
  branches: Array<Branch>;
  total: number;
}
