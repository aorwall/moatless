import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import type { WorkspaceProvisionPlan } from './provision-plan.types'
import type { ExecutionTarget } from './workspace-runtime.types'

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
   * Provision a new workspace or ensure existing one is ready
   *
   * @param taskId - Task ID for the workspace
   * @param repositoryId - Repository ID (may be needed for K8s clone URL resolution)
   * @param plan - Provision plan with mode, branch, etc.
   * @returns ExecutionTarget with environment-specific configuration
   */
  provision(
    taskId: string,
    repositoryId: string,
    plan: WorkspaceProvisionPlan,
  ): Promise<ExecutionTarget>

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
   * @param target - Execution target to check
   * @returns true if healthy, false if stale/unreachable
   */
  isHealthy(target: ExecutionTarget): Promise<boolean>

  /**
   * Validate workspace exists and is properly configured
   *
   * More thorough than isHealthy - used for initial validation:
   * - Local: Check path exists and is valid git repo
   * - Kubernetes: Check pod exists and is in Running phase
   *
   * @param target - Execution target to validate
   * @returns true if valid, false otherwise
   */
  validate(target: ExecutionTarget): Promise<boolean>

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
   * Clear error state for a failed workspace
   *
   * Called before reprovisioning after a failure:
   * - Local: No-op (no external resources)
   * - Kubernetes: Delete failed pod so a new one can be created
   *
   * @param taskId - Task ID
   * @param target - Execution target with error
   */
  clearError(taskId: string, target: ExecutionTarget): Promise<void>

  // =============================================================================
  // Driver Factory
  // =============================================================================

  /**
   * Create an environment driver for the workspace
   *
   * - Local: Creates LocalEnvironmentDriver with workspace path
   * - Kubernetes: Creates KubernetesEnvironmentDriver with pod info
   *
   * @param taskId - Task ID
   * @param target - Execution target with configuration
   * @returns Environment driver for file ops, commands, agents
   */
  createDriver(taskId: string, target: ExecutionTarget): BaseEnvironmentDriver
}
