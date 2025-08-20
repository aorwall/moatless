// Re-export all types from workspace schemas

// Re-export ShellCommandResult from shell-command schemas
export type { ShellCommandResult } from '../schemas/shell-command.schemas.js'
// Re-export ServerStatus from task schemas (it's defined there)
export type { ServerStatus } from '../schemas/task.schemas.js'
export type {
  AgentState,
  BaseWorkspace,
  DockerWorkspace,
  EnvironmentStatus,
  FileTreeNode,
  FileTreeWithChangesResponse,
  GitChangeStatus,
  LocalWorkspace,
  SyncWorkspaceResponse,
  Workspace,
  WorkspaceCapabilities,
  WorkspaceChange,
  WorkspaceChangeStatus,
  WorkspaceExistence,
  WorkspaceFile,
  WorkspaceFileSearchResult,
  WorkspaceSetupRequest,
  WorkspaceSetupResponse,
  WorkspaceStatusResponse,
  WorkspaceType,
} from '../schemas/workspace.schemas.js'

// Re-export functions from schemas
export { isDockerWorkspace, isLocalWorkspace } from '../schemas/workspace.schemas.js'
