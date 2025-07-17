// Re-export all types from workspace schemas

// Re-export CommandResult and ServerStatus from task schemas (they're defined there)
export type { CommandResult, ServerStatus } from '../schemas/task.schemas.js'
export type {
  BaseWorkspace,
  DockerWorkspace,
  FileTreeNode,
  FileTreeWithChangesResponse,
  GitChangeStatus,
  LocalWorkspace,
  SyncWorkspaceResponse,
  Workspace,
  WorkspaceCapabilities,
  WorkspaceChange,
  WorkspaceFile,
  WorkspaceFileSearchResult,
  WorkspaceSetupRequest,
  WorkspaceSetupResponse,
  WorkspaceState,
  WorkspaceStatus,
  WorkspaceStatusResponse,
  WorkspaceType,
} from '../schemas/workspace.schemas.js'

// Re-export functions from schemas
export { isDockerWorkspace, isLocalWorkspace } from '../schemas/workspace.schemas.js'
