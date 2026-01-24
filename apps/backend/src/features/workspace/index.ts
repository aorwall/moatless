/**
 * Workspace feature exports
 * This module manages workspace provisioning, runtime state, and execution
 */

// API Routes
export { registerFileRoutes, registerWorkspaceRoutes } from './api'
// Repository
export { WorkspaceRuntimeRepository } from './db/workspace-runtime.repository'
// Drivers
export type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  RunCommandOptions,
} from './drivers/base-environment-driver'
// Services (ordered by dependency hierarchy)
export { ProvisionPlanFactory } from './services/provision-plan-factory.service'
export { WorkspaceCoordinator } from './services/workspace-coordinator.service'
export { WorkspaceRuntimeManager } from './services/workspace-runtime-manager.service'

// Types
export type { WorkspaceProvisionPlan } from './types/provision-plan.types'
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
export {
  bumpSnapshotRevision,
  createDockerWorkspaceSnapshot,
  createKubernetesWorkspaceSnapshot,
  createWorkspaceSnapshotFromTarget,
  isDockerExecutionTarget,
  isKubernetesExecutionTarget,
  isWorkspaceAvailable,
} from './types/workspace-runtime.types'
