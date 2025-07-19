// Re-export all types from workspace schemas

// Re-export CommandResult and ServerStatus from task schemas (they're defined there)
export type { CommandResult, ServerStatus } from '../schemas/task.schemas.js'
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
