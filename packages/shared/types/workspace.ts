export enum WorkspaceStatus {
  NOT_SETUP = 'not_setup',
  SETTING_UP = 'setting_up',
  IDLE = 'idle',
  RUNNING = 'running',
}

export interface BaseWorkspace {
  id: string
  type: 'local' | 'docker'
  repositoryId: string
  status: 'initializing' | 'ready' | 'error' | 'cleaning'
  createdAt: Date
  error?: string
}

export interface LocalWorkspace extends BaseWorkspace {
  type: 'local'
  path: string
  branch?: string
  originalBranch?: string
}

export interface DockerWorkspace extends BaseWorkspace {
  type: 'docker'
  containerId: string
  port?: number
  image?: string
}

export type Workspace = LocalWorkspace | DockerWorkspace

export function isLocalWorkspace(workspace: Workspace): workspace is LocalWorkspace {
  return workspace.type === 'local'
}

export function isDockerWorkspace(workspace: Workspace): workspace is DockerWorkspace {
  return workspace.type === 'docker'
}

export interface WorkspaceFile {
  path: string
  content: string
  language?: string
}

export interface WorkspaceChange {
  type: 'create' | 'modify' | 'delete'
  path: string
  oldContent?: string
  newContent?: string
}

export interface CommandResult {
  command: string
  output: string
  exitCode: number
  durationMs: number
}

export interface ServerStatus {
  running: boolean
  configured: boolean
  url?: string
  pid?: number
  uptimeSeconds: number
}

export interface WorkspaceFileSearchResult {
  path: string
  matches: number
}

export interface WorkspaceCapabilities {
  canListFiles: boolean
  canReadWriteFiles: boolean
  canSearchFiles: boolean
  canExecuteCode: boolean
}
