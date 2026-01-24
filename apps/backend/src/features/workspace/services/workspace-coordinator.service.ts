import type { EnvironmentStatus, WorkspaceStatusResponse } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { SettingsService } from '../../../services/settings.service'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import {
  getWorkspacePathFromTarget,
  isWorkspaceAvailable as isTargetAvailable,
  type KubernetesExecutionTarget,
  type WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'
import { WorkspaceRuntimeManager } from './workspace-runtime-manager.service'

/**
 * WorkspaceCoordinator - High-level workspace orchestrator
 *
 * Manages complete workspace lifecycle including coordination of
 * preview servers, agents, and runtime state.
 *
 * Responsibilities:
 * - Provide unified API for workspace operations
 * - Orchestrate cleanup across agent and runtime state
 * - Delegate provisioning and state management to RuntimeManager
 * - Sync preview server state from sandbox
 *
 * Design principle:
 * - This is the HIGH-LEVEL orchestrator
 * - Depends on RuntimeManager
 * - No circular dependencies (managers depend on RuntimeManager, not this)
 */
@injectable()
export class WorkspaceCoordinator {
  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(SettingsService) private readonly settingsService: SettingsService,
  ) {}

  /**
   * Ensure workspace is provisioned and ready
   * Syncs preview server state from sandbox if configured
   * Delegates to RuntimeManager
   *
   * Note: The preview server is auto-started by the container entrypoint.
   * We query the sandbox's actual preview status to sync the backend snapshot,
   * since sandbox events don't automatically update the backend's snapshot.
   */
  async ensureWorkspace(taskId: string, repositoryId?: string): Promise<WorkspaceHandle> {
    const handle = await this.runtimeManager.ensureWorkspace(taskId, repositoryId)
    return handle
  }

  /**
   * Get workspace runtime snapshot
   * Delegates to RuntimeManager
   */
  async getSnapshot(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    return await this.runtimeManager.getSnapshot(taskId)
  }

  /**
   * Check if workspace is available and ready for use
   * Checks if the execution target (K8s pod) is running and ready
   */
  async isWorkspaceAvailable(taskId: string): Promise<boolean> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
    if (!snapshot) {
      return false
    }
    // Check if execution target is available (pod is running)
    return isTargetAvailable(snapshot.executionTarget)
  }

  /**
   * Check if an agent is actually running by querying the worker.
   * Used to validate database state before starting a new agent.
   * Delegates to RuntimeManager.
   *
   * @param taskId - Task ID to check
   * @returns true if agent is confirmed running, false otherwise
   */
  async isAgentActuallyRunning(taskId: string): Promise<boolean> {
    return await this.runtimeManager.isAgentActuallyRunning(taskId)
  }

  /**
   * Update agent execution state in runtime snapshot.
   * Delegates to RuntimeManager.
   *
   * @param taskId - Task ID
   * @param updates - Partial agent state updates
   */
  async updateAgentState(
    taskId: string,
    updates: Partial<{ state: 'idle' | 'running'; sessionId?: string; startedAt?: Date }>,
  ): Promise<void> {
    await this.runtimeManager.updateAgentState(taskId, updates)
  }

  /**
   * Delete workspace snapshot
   * Delegates to RuntimeManager
   * @deprecated Use deleteWorkspace() for complete cleanup
   */
  async deleteSnapshot(taskId: string): Promise<void> {
    await this.runtimeManager.deleteSnapshot(taskId)
  }

  /**
   * Complete workspace cleanup - orchestrates all cleanup operations
   *
   * Workflow:
   * 1. Stop any running agents
   * 2. Stop preview servers
   * 3. Clean up workspace artifacts (clones, worktrees)
   * 4. Delete workspace snapshot
   *
   * This is the single entry point for full workspace cleanup.
   */
  async deleteWorkspace(taskId: string): Promise<void> {
    logger.info({ taskId }, 'WorkspaceCoordinator: Starting complete workspace cleanup')

    try {
      // 1. Stop any running agents (via RuntimeManager)
      try {
        await this.runtimeManager.stopAgentAndInvalidate(taskId)
        logger.debug({ taskId }, 'WorkspaceCoordinator: Agent stopped and driver invalidated')
      } catch (error) {
        logger.warn(
          {
            taskId,
            error: error instanceof Error ? error.message : String(error),
          },
          'WorkspaceCoordinator: Failed to stop agent during cleanup',
        )
        // Continue cleanup even if agent stop fails
      }

      // 2. Clean up workspace artifacts (clones, worktrees, etc.)
      try {
        await this.runtimeManager.cleanupWorkspaceArtifacts(taskId)
        logger.debug({ taskId }, 'WorkspaceCoordinator: Workspace artifacts cleaned up')
      } catch (error) {
        logger.warn(
          {
            taskId,
            error: error instanceof Error ? error.message : String(error),
          },
          'WorkspaceCoordinator: Failed to clean up workspace artifacts',
        )
        // Continue cleanup even if artifact cleanup fails
      }

      // 3. Delete workspace snapshot
      await this.runtimeManager.deleteSnapshot(taskId)
      logger.debug({ taskId }, 'WorkspaceCoordinator: Workspace snapshot deleted')

      logger.info({ taskId }, 'WorkspaceCoordinator: Complete workspace cleanup finished')
    } catch (error) {
      logger.error(
        { err: error as Error, taskId },
        'WorkspaceCoordinator: Failed to complete workspace cleanup',
      )
      throw error
    }
  }

  /**
   * Get workspace status for API response
   * Maps runtime snapshot to WorkspaceStatusResponse format
   */
  async getWorkspaceStatus(taskId: string): Promise<WorkspaceStatusResponse> {
    // Validate task exists first
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Get workspace snapshot with live environment validation
    const snapshot = await this.runtimeManager.getValidatedSnapshot(taskId)
    if (!snapshot) {
      // No snapshot exists yet - workspace hasn't been provisioned
      // Return minimal status indicating workspace needs setup
      return {
        workspaceState: 'missing',
        workspacePath: '',
        environmentType: 'local',
        environmentStatus: 'not_created',
        environmentCreatedAt: new Date().toISOString(),
        configOutdated: false,
        agentState: 'none',
        gitStatus: undefined,
        capabilities: {
          fileOperations: false,
          commandExecution: false,
          serverManagement: false,
        },
        isRunning: false,
        workspaceReady: false,
        canExecute: false,
      }
    }

    // Map snapshot to status response
    const repository = await this.repoService.getRepositoryById(snapshot.repositoryId)

    // Check if repository config is outdated
    const k8sTarget = snapshot.executionTarget as KubernetesExecutionTarget
    const envCreatedAt = k8sTarget.lastHealthCheck || new Date(snapshot.createdAt)
    const configOutdated = repository ? repository.updatedAt > envCreatedAt : false

    // Determine workspace path from execution target
    const workspacePath = getWorkspacePathFromTarget(snapshot.executionTarget)

    // Get execution mode from system settings (not from repository config)
    const executionMode = this.settingsService.getWorkspaceExecutionMode()

    // Check if workspace is available
    const workspaceAvailable = isTargetAvailable(snapshot.executionTarget)

    // Determine environment status - use unified status from execution target
    // If there's an error, always return 'error'
    // Otherwise, use the status directly from the target
    const getEnvironmentStatus = (): EnvironmentStatus => {
      if (snapshot.executionTarget.lastError) {
        return 'error'
      }
      // Status is now unified on the execution target
      return snapshot.executionTarget.status ?? 'not_created'
    }

    const environmentStatus = getEnvironmentStatus()

    const status: WorkspaceStatusResponse = {
      workspaceState: 'exists',
      workspacePath,
      environmentType: executionMode,
      environmentStatus,
      environmentError: snapshot.executionTarget.lastError,
      environmentCreatedAt: snapshot.createdAt.toISOString(),
      repositoryUpdatedAt: repository?.updatedAt.toISOString(),
      configOutdated,
      agentState: snapshot.agent?.state === 'running' ? 'running' : 'none',
      agentType: task.agentType,
      capabilities: {
        fileOperations: workspaceAvailable,
        commandExecution: workspaceAvailable,
        serverManagement:
          workspaceAvailable &&
          Boolean(
            repository?.previewConfig?.serverStartCommands &&
              repository.previewConfig.serverStartCommands.length > 0,
          ),
      },
      isRunning:
        environmentStatus === 'ready' &&
        (snapshot.preview.status === 'started' || snapshot.agent?.state === 'running'),
      workspaceReady: environmentStatus === 'ready',
      canExecute: environmentStatus === 'ready',
    }

    return status
  }

  /**
   * Retry workspace provisioning after a failure
   * Clears the error state and provisions a new workspace immediately
   */
  async retryWorkspace(taskId: string): Promise<{ success: boolean; message: string }> {
    // Validate task exists
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Clear the error state (deletes pod and snapshot)
    await this.runtimeManager.clearWorkspaceError(taskId)

    // Provision new workspace immediately
    try {
      await this.ensureWorkspace(taskId, task.repositoryId)
      return {
        success: true,
        message: 'Workspace reprovisioned successfully.',
      }
    } catch (error) {
      return {
        success: false,
        message: `Failed to reprovision workspace: ${(error as Error).message}`,
      }
    }
  }
}
