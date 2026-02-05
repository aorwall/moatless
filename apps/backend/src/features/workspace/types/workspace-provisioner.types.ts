import type { EnvironmentStatus } from '@moatless/types'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import type { WorkspaceProvisionPlan } from './provision-plan.types'
import type { ExecutionTarget } from './workspace-runtime.types'

/**
 * Result of getStatus() call - provides detailed workspace status for logging
 */
export interface WorkspaceStatusResult {
  status: EnvironmentStatus
  containerName?: string | undefined // For Docker
  podName?: string | undefined // For K8s
  lastError?: string | undefined // When status is 'error'
}

/**
 * WorkspaceProvisioner - Interface for workspace lifecycle management
 *
 * This interface abstracts environment-specific logic (local vs Kubernetes)
 * from the WorkspaceRuntimeManager orchestration layer.
 *
 * Responsibilities:
 * - Provisioning: Create and configure workspace environments
 * - Health checking: Validate workspace is operational (for cache invalidation)
 * - Validation: Check if workspace exists and is valid
 * - Agent status: Query if agent is running in the environment
 * - Cleanup: Remove workspace artifacts
 * - Driver creation: Factory method for environment-specific drivers
 *
 * Implementations:
 * - LocalWorkspaceProvisioner: Local filesystem workspaces
 * - KubernetesWorkspaceProvisioner: Kubernetes pod workspaces
 */
export interface WorkspaceProvisioner {
  // =============================================================================
  // Provisioning
  // =============================================================================

  /**
   * Create an initializing execution target before provisioning begins.
   * Used to save a snapshot and emit events immediately, before the actual
   * resource (pod/container) is created.
   *
   * @param taskId - Task ID for the workspace
   * @param plan - Provision plan with resolved repoConfig, credentials, etc.
   * @returns ExecutionTarget with 'initializing' status (without resource names)
   */
  createInitializingTarget(taskId: string, plan: WorkspaceProvisionPlan): ExecutionTarget

  /**
   * Provision a new workspace or ensure existing one is ready
   *
   * @param taskId - Task ID for the workspace
   * @param plan - Provision plan with resolved repoConfig, credentials, etc.
   * @returns ExecutionTarget with environment-specific configuration
   */
  provision(taskId: string, plan: WorkspaceProvisionPlan): Promise<ExecutionTarget>

  // =============================================================================
  // Health & Validation
  // =============================================================================

  /**
   * Check if workspace is healthy and operational
   *
   * Used by driver cache to detect stale drivers:
   * - Local: Check if workspace path exists and is accessible
   * - Kubernetes: HTTP health check to pod endpoint
   *
   * @param taskId - Task ID for pod lookup
   * @param target - Execution target to check
   * @returns true if healthy, false if stale/unreachable
   */
  isHealthy(taskId: string, target: ExecutionTarget): Promise<boolean>

  /**
   * Get workspace status for logging and decision making
   *
   * Returns detailed status information instead of boolean:
   * - Docker: Check container status via DockerService
   * - Kubernetes: Check pod phase via KubernetesService
   *
   * Use this method to understand WHY a workspace isn't ready
   * and log appropriate context.
   *
   * @param taskId - Task ID for pod lookup
   * @param target - Execution target to check
   * @returns WorkspaceStatusResult with status and identifying info
   */
  getStatus(
    taskId: string,
    target: ExecutionTarget,
  ): WorkspaceStatusResult | Promise<WorkspaceStatusResult>

  // =============================================================================
  // Agent Status
  // =============================================================================

  /**
   * Check if an agent is currently running in the workspace
   *
   * - Local: Check in-memory/database state (agents run in same process)
   * - Kubernetes: Query /agent-status endpoint on worker pod
   *
   * @param taskId - Task ID to check
   * @param target - Execution target
   * @returns true if agent is confirmed running
   */
  isAgentRunning(taskId: string, target: ExecutionTarget): Promise<boolean>

  // =============================================================================
  // Cleanup
  // =============================================================================

  /**
   * Clean up workspace artifacts
   *
   * - Local: Remove worktree/clone directory via materializer (requires plan)
   * - Kubernetes: Delete pod from cluster
   *
   * @param taskId - Task ID
   * @param target - Execution target to clean up
   * @param plan - Optional provision plan (required for local cleanup)
   */
  cleanup(taskId: string, target: ExecutionTarget, plan?: WorkspaceProvisionPlan): Promise<void>

  /**
   * Stop sandbox without deleting persistent data
   *
   * Stops the running sandbox (pod/container) but preserves persistent storage:
   * - Docker: Stops and removes container (no separate persistence)
   * - Kubernetes: Deletes Pod + Service, keeps PVC (files) and Secret (credentials)
   *
   * Use this for workspace restart and error recovery scenarios.
   *
   * @param taskId - Task ID
   * @param target - Execution target to stop
   */
  stopSandbox(taskId: string, target: ExecutionTarget): Promise<void>

  // =============================================================================
  // Driver Factory
  // =============================================================================

  /**
   * Create an environment driver for the workspace
   *
   * Creates a RemoteEnvironmentDriver configured for the execution target
   * (Docker container or Kubernetes pod) with appropriate URL resolution.
   *
   * @param taskId - Task ID
   * @param target - Execution target with configuration
   * @returns Environment driver for file ops, commands, agents
   */
  createDriver(taskId: string, target: ExecutionTarget): BaseEnvironmentDriver
}
