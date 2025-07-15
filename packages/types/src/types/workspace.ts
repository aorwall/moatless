// Re-export types from schemas (excluding WorkspaceStatus to avoid conflict)

// Re-export CommandResult and ServerStatus from task schemas (they're defined there)
export type { CommandResult, ServerStatus } from '../schemas/task.schemas.js'
export type {
  BaseWorkspace,
  DockerWorkspace,
  LocalWorkspace,
  Workspace,
  WorkspaceCapabilities,
  WorkspaceChange,
  WorkspaceFile,
  WorkspaceFileSearchResult,
  WorkspaceState,
  WorkspaceType,
} from '../schemas/workspace.schemas.js'
// Re-export functions from schemas
export { isDockerWorkspace, isLocalWorkspace } from '../schemas/workspace.schemas.js'

// Keep the enum for backward compatibility (this takes precedence over the schema type)
export enum WorkspaceStatus {
  NOT_SETUP = 'not_setup',
  SETTING_UP = 'setting_up',
  IDLE = 'idle',
  RUNNING = 'running',
}
