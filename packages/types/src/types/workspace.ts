// Re-export all types from workspace schemas

// Re-export ServerStatus from preview schemas (it's defined there)
export type { ServerStatus } from '../schemas/preview.schemas.js'
// Re-export ShellCommandResult from shell-command schemas
export type { ShellCommandResult } from '../schemas/shell-command.schemas.js'
export type {
  AgentState,
  BaseWorkspace,
  DockerWorkspace,
  EnvironmentStatus,
  FileTreeNode,
  FileTreeWithChangesResponse,
  LocalWorkspace,
  Workspace,
  WorkspaceCapabilities,
  WorkspaceChange,
  WorkspaceChangeStatus,
  WorkspaceExistence,
  WorkspaceFile,
  WorkspaceFileSearchResult,
  WorkspaceStatusResponse,
  WorkspaceType,
} from '../schemas/workspace.schemas.js'
