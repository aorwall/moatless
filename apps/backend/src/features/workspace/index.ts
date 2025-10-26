/**
 * Workspace feature exports
 * This module manages workspace provisioning, runtime state, and execution
 */

// API Routes
export { registerAgentRoutes, registerPreviewRoutes } from './api'

export type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  PreviewConfiguration,
  RunCommandOptions,
} from './drivers/base-environment-driver'
// Drivers
export { LocalEnvironmentDriver } from './drivers/local-environment-driver'
export { RemoteWorkerEnvironment } from './drivers/remote-worker-environment'
// Services
export { AgentController } from './services/agent-controller.service'
export { FileGateway } from './services/file-gateway.service'
export { PreviewManager } from './services/preview-manager.service'
export { WorkspaceCoordinator } from './services/workspace-coordinator.service'
export { WorkspaceMaterializer } from './services/workspace-materializer.service'

// Storage
export { WorkspaceRuntimeStorage } from './storage/workspace-runtime.storage'
export type { CredentialBundle, WorkspaceProvisionPlan } from './types/provision-plan.types'
export type { WorkspaceHandle } from './types/workspace-handle.types'
// Types
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
