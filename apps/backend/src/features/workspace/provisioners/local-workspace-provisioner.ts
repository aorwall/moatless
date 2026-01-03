import { inject, injectable } from 'inversify'
import {
  provisionerCleanupTotal,
  provisionerDriverCreatedTotal,
  provisionerHealthCheckDurationSeconds,
  provisionerHealthCheckTotal,
  provisionerValidateTotal,
  workspaceProvisionDurationSeconds,
  workspaceProvisionTotal,
} from '../../../services/metrics'
import { pathExists } from '../../../utils/filesystem'
import { logger } from '../../../utils/logger'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import { LocalEnvironmentDriver } from '../drivers/local-environment-driver'
import {
  WorkspaceMaterializer,
  WorkspacePathMissingError,
} from '../services/workspace-materializer.service'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceProvisioner } from '../types/workspace-provisioner.types'
import {
  type ExecutionTarget,
  isLocalExecutionTarget,
  type LocalExecutionTarget,
} from '../types/workspace-runtime.types'

/**
 * LocalWorkspaceProvisioner - Handles local workspace lifecycle
 *
 * Implements WorkspaceProvisioner for local filesystem workspaces.
 * Delegates physical operations (clone, worktree) to WorkspaceMaterializer.
 *
 * Responsibilities:
 * - Provision workspaces via direct path, worktree, or clone
 * - Validate workspace paths exist and are valid git repos
 * - Check workspace health (path accessibility)
 * - Clean up workspace artifacts
 * - Create LocalEnvironmentDriver instances
 */
@injectable()
export class LocalWorkspaceProvisioner implements WorkspaceProvisioner {
  constructor(
    @inject(WorkspaceMaterializer) private readonly materializer: WorkspaceMaterializer,
  ) {}

  /**
   * Provision a local workspace
   *
   * Delegates to WorkspaceMaterializer to apply the provision plan:
   * - direct: Uses repository path directly
   * - worktree: Creates git worktree
   * - clone: Clones repository to new directory
   *
   * @returns LocalExecutionTarget with workspace path
   */
  async provision(
    taskId: string,
    repositoryId: string,
    plan: WorkspaceProvisionPlan,
  ): Promise<ExecutionTarget> {
    const startTime = Date.now()
    logger.debug(
      { taskId, repositoryId, mode: plan.mode },
      'LocalWorkspaceProvisioner: Provisioning workspace',
    )

    try {
      const handle = await this.materializer.applyPlan(taskId, plan)

      const target: LocalExecutionTarget = {
        mode: 'local',
        workspacePath: handle.workspacePath,
        isAvailable: true,
        lastValidatedAt: new Date(),
      }

      // Track success metrics
      const durationSeconds = (Date.now() - startTime) / 1000
      workspaceProvisionTotal.labels({ mode: plan.mode, status: 'success' }).inc()
      workspaceProvisionDurationSeconds.labels({ mode: plan.mode }).observe(durationSeconds)

      logger.info(
        { taskId, workspacePath: handle.workspacePath, mode: plan.mode },
        'LocalWorkspaceProvisioner: Workspace provisioned successfully',
      )

      return target
    } catch (error) {
      // Track error metrics
      workspaceProvisionTotal.labels({ mode: plan.mode, status: 'error' }).inc()

      // Handle workspace path missing gracefully
      if (error instanceof WorkspacePathMissingError) {
        logger.warn(
          { taskId, path: error.path, error: error.message },
          'LocalWorkspaceProvisioner: Workspace path missing, marking as unavailable',
        )

        const target: LocalExecutionTarget = {
          mode: 'local',
          workspacePath: error.path,
          isAvailable: false,
          lastError: error.message,
          lastValidatedAt: new Date(),
        }

        return target
      }

      // Re-throw other errors
      throw error
    }
  }

  /**
   * Check if workspace is healthy (path exists and accessible)
   *
   * Used by driver cache to detect stale drivers.
   * For local workspaces, this is a simple path existence check.
   */
  async isHealthy(target: ExecutionTarget): Promise<boolean> {
    const startTime = Date.now()

    if (!isLocalExecutionTarget(target)) {
      logger.warn('LocalWorkspaceProvisioner: isHealthy called with non-local target')
      provisionerHealthCheckTotal.labels({ mode: 'local', result: 'error' }).inc()
      return false
    }

    const exists = await pathExists(target.workspacePath)

    // Track metrics
    const durationSeconds = (Date.now() - startTime) / 1000
    provisionerHealthCheckDurationSeconds.labels({ mode: 'local' }).observe(durationSeconds)
    provisionerHealthCheckTotal
      .labels({ mode: 'local', result: exists ? 'healthy' : 'unhealthy' })
      .inc()

    if (!exists) {
      logger.debug(
        { workspacePath: target.workspacePath },
        'LocalWorkspaceProvisioner: Workspace path not found',
      )
    }

    return exists
  }

  /**
   * Validate workspace exists and is properly configured
   *
   * More thorough than isHealthy - checks git repo validity.
   */
  async validate(target: ExecutionTarget): Promise<boolean> {
    if (!isLocalExecutionTarget(target)) {
      logger.warn('LocalWorkspaceProvisioner: validate called with non-local target')
      provisionerValidateTotal.labels({ mode: 'local', result: 'error' }).inc()
      return false
    }

    const isValid = await this.materializer.validateWorkspace(target.workspacePath)

    // Track metrics
    provisionerValidateTotal.labels({ mode: 'local', result: isValid ? 'valid' : 'invalid' }).inc()

    if (!isValid) {
      logger.warn(
        { workspacePath: target.workspacePath },
        'LocalWorkspaceProvisioner: Workspace validation failed',
      )
    }

    return isValid
  }

  /**
   * Check if an agent is running
   *
   * For local workspaces, agents run in the same process.
   * We can't directly check from here - caller should check runtime snapshot.
   * This method always returns false as a safe default.
   */
  isAgentRunning(_taskId: string, _target: ExecutionTarget): Promise<boolean> {
    // Local agents are tracked in WorkspaceRuntimeSnapshot.agent.state
    // The RuntimeManager should check the snapshot directly
    // Returning false is safe - caller will verify from snapshot
    return Promise.resolve(false)
  }

  /**
   * Clean up workspace artifacts
   *
   * Delegates to WorkspaceMaterializer for physical cleanup.
   * Requires the original provision plan to determine cleanup actions.
   */
  async cleanup(
    taskId: string,
    target: ExecutionTarget,
    plan?: WorkspaceProvisionPlan,
  ): Promise<void> {
    if (!isLocalExecutionTarget(target)) {
      logger.warn('LocalWorkspaceProvisioner: cleanup called with non-local target')
      return
    }

    if (!plan) {
      logger.warn(
        { taskId },
        'LocalWorkspaceProvisioner: No provision plan provided for cleanup, skipping',
      )
      return
    }

    logger.debug({ taskId, mode: plan.mode }, 'LocalWorkspaceProvisioner: Cleaning up workspace')

    try {
      await this.materializer.cleanupWorkspace(taskId, plan)
      provisionerCleanupTotal.labels({ mode: 'local', status: 'success' }).inc()
      logger.info({ taskId }, 'LocalWorkspaceProvisioner: Workspace cleaned up')
    } catch (error) {
      provisionerCleanupTotal.labels({ mode: 'local', status: 'error' }).inc()
      logger.error(
        { taskId, error: (error as Error).message },
        'LocalWorkspaceProvisioner: Failed to clean up workspace',
      )
      // Don't throw - cleanup should be best-effort
    }
  }

  /**
   * Clear error state for a failed workspace
   *
   * For local workspaces, there's no external resource to clean up.
   * The snapshot will be deleted by RuntimeManager.
   */
  clearError(_taskId: string, _target: ExecutionTarget): Promise<void> {
    // Local workspaces don't have external resources like pods
    // Just log and return - snapshot cleanup is handled by RuntimeManager
    logger.debug('LocalWorkspaceProvisioner: clearError is no-op for local workspaces')
    return Promise.resolve()
  }

  /**
   * Create a LocalEnvironmentDriver for the workspace
   */
  createDriver(taskId: string, target: ExecutionTarget): BaseEnvironmentDriver {
    if (!isLocalExecutionTarget(target)) {
      throw new Error('LocalWorkspaceProvisioner: Cannot create driver for non-local target')
    }

    provisionerDriverCreatedTotal.labels({ mode: 'local' }).inc()
    return new LocalEnvironmentDriver(taskId, target.workspacePath)
  }
}
