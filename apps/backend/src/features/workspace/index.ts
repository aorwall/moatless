/**
 * Workspace feature exports
 * This module manages workspace provisioning, runtime state, and execution
 */

// API Routes
export { registerPreviewRoutes } from './api'

// Drivers
export type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  PreviewConfiguration,
  RunCommandOptions,
} from './drivers/base-environment-driver'
export { LocalEnvironmentDriver } from './drivers/local-environment-driver'
export { RemoteWorkerEnvironment } from './drivers/remote-worker-environment'
export { FileGateway } from './services/file-gateway.service'
export { PreviewManager } from './services/preview-manager.service'
export { WorkspaceCoordinator } from './services/workspace-coordinator.service'
export { WorkspaceMaterializer } from './services/workspace-materializer.service'
// Services (ordered by dependency hierarchy)
export { WorkspaceRuntimeManager } from './services/workspace-runtime-manager.service'

// Storage
export { WorkspaceRuntimeStorage } from './storage/workspace-runtime.storage'

// Types
export type { CredentialBundle, WorkspaceProvisionPlan } from './types/provision-plan.types'
export type {
  BaseWorkspaceHandle,
  WorkspaceHandle,
  WorkspaceHandlePreviewState,
} from './types/workspace-handle.types'
export type {
  AgentState,
  ExecutionTarget,
  PreviewState,
  WorkspaceLock,
  WorkspaceRuntimeSnapshot,
} from './types/workspace-runtime.types'
export { bumpConfigRevision, createWorkspaceSnapshot } from './types/workspace-runtime.types'

// Utils
export * from './utils/git.utils'
