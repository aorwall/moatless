import type { WorkspaceStatusResponse } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { SettingsService } from '../../../services/settings.service'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import type { WorkspaceRuntimeSnapshot } from '../types/workspace-runtime.types'
import { PreviewManager } from './preview-manager.service'
import { WorkspaceRuntimeManager } from './workspace-runtime-manager.service'

/**
 * WorkspaceCoordinator - High-level workspace orchestrator
 *
 * Manages complete workspace lifecycle including coordination of
 * preview servers, agents, and runtime state.
 *
 * Responsibilities:
 * - Provide unified API for workspace operations
 * - Orchestrate cleanup across preview, agent, and runtime state
 * - Delegate provisioning and state management to RuntimeManager
 * - Coordinate preview and agent operations
 *
 * Design principle:
 * - This is the HIGH-LEVEL orchestrator
 * - Depends on PreviewManager, AgentController, and RuntimeManager
 * - No circular dependencies (managers depend on RuntimeManager, not this)
 */
@injectable()
export class WorkspaceCoordinator {
  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(PreviewManager) private readonly previewManager: PreviewManager,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(SettingsService) private readonly settingsService: SettingsService,
  ) {}

  /**
   * Ensure workspace is provisioned and ready
   * Automatically starts preview server if configured in repository
   * Delegates to RuntimeManager
   */
  async ensureWorkspace(taskId: string, repositoryId?: string): Promise<WorkspaceHandle> {
    // 1. Ensure workspace is provisioned
    const handle = await this.runtimeManager.ensureWorkspace(taskId, repositoryId)

    // 2. Check if repository has preview server configured
    const repoConfig = await this.repoService.getRepositoryById(handle.repositoryId)

    // 3. Auto-start preview if configured and not running
    if (
      repoConfig?.previewConfig?.serverStartCommands &&
      repoConfig.previewConfig.serverStartCommands.length > 0
    ) {
      if (handle.preview.status !== 'started' && handle.preview.status !== 'starting') {
        logger.info(
          'WorkspaceCoordinator: Initiating preview server start in background (configured in repository)',
          {
            taskId,
            repositoryId: handle.repositoryId,
          },
        )
        // Update state to 'starting' immediately to prevent race conditions
        // This ensures concurrent calls to ensureWorkspace see the updated status
        await this.runtimeManager.updatePreviewState(taskId, { status: 'starting' })

        // Start preview in background - don't block workspace provisioning
        // Errors are handled by PreviewManager and reflected in preview state
        this.previewManager.start(taskId, repositoryId).catch((error) => {
          // Error already logged by PreviewManager, just ensure it doesn't propagate
          logger.debug('WorkspaceCoordinator: Preview auto-start error handled in background', {
            taskId,
            error: error instanceof Error ? error.message : String(error),
          })
        })
      } else {
        logger.debug(
          'WorkspaceCoordinator: Preview server already started or starting, skipping auto-start',
          {
            taskId,
          },
        )
      }
    }

    // 4. Return handle
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
   * 3. Delete workspace snapshot
   *
   * This is the single entry point for full workspace cleanup.
   */
  async deleteWorkspace(taskId: string): Promise<void> {
    logger.info('WorkspaceCoordinator: Starting complete workspace cleanup', { taskId })

    try {
      // 1. Stop any running agents
      try {
        await this.runtimeManager.stopAgent(taskId)
        logger.debug('WorkspaceCoordinator: Agent stopped', { taskId })
      } catch (error) {
        logger.warn('WorkspaceCoordinator: Failed to stop agent during cleanup', {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        })
        // Continue cleanup even if agent stop fails
      }

      // 2. Stop preview server
      try {
        await this.previewManager.stop(taskId)
        logger.debug('WorkspaceCoordinator: Preview server stopped', { taskId })
      } catch (error) {
        logger.warn('WorkspaceCoordinator: Failed to stop preview during cleanup', {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        })
        // Continue cleanup even if preview stop fails
      }

      // 3. Delete workspace snapshot
      await this.runtimeManager.deleteSnapshot(taskId)
      logger.debug('WorkspaceCoordinator: Workspace snapshot deleted', { taskId })

      logger.info('WorkspaceCoordinator: Complete workspace cleanup finished', { taskId })
    } catch (error) {
      logger.error('WorkspaceCoordinator: Failed to complete workspace cleanup', error as Error, {
        taskId,
      })
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

    // Get or create workspace snapshot
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
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
    const envCreatedAt = snapshot.executionTarget.lastValidatedAt || new Date(snapshot.createdAt)
    const configOutdated = repository ? repository.updatedAt > envCreatedAt : false

    // Determine workspace path from execution target
    const workspacePath =
      snapshot.executionTarget.mode === 'local' ? snapshot.executionTarget.workspacePath : ''

    // Get execution mode from system settings (not from repository config)
    const executionMode = this.settingsService.getWorkspaceExecutionMode()

    const status: WorkspaceStatusResponse = {
      workspaceState: 'exists',
      workspacePath,
      environmentType: executionMode,
      environmentStatus: snapshot.executionTarget.lastError ? 'error' : 'ready',
      environmentError: snapshot.executionTarget.lastError,
      environmentCreatedAt: snapshot.createdAt.toISOString(),
      repositoryUpdatedAt: repository?.updatedAt.toISOString(),
      configOutdated,
      agentState: snapshot.agent?.state === 'running' ? 'running' : 'none',
      agentType: task.agentType,
      capabilities: {
        fileOperations: true,
        commandExecution: true,
        serverManagement: Boolean(
          repository?.previewConfig?.serverStartCommands &&
            repository.previewConfig.serverStartCommands.length > 0,
        ),
      },
      isRunning: snapshot.preview.status === 'started' || snapshot.agent?.state === 'running',
      workspaceReady: !snapshot.executionTarget.lastError,
      canExecute: !snapshot.executionTarget.lastError,
    }

    return status
  }
}
