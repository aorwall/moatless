import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { AgentController } from '../../agents/services/agent-controller.service'
import { RepositoryService } from '../../repositories/index'
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
    @inject(AgentController) private readonly agentController: AgentController,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
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
      if (handle.preview.status !== 'started') {
        logger.info(
          'WorkspaceCoordinator: Auto-starting preview server (configured in repository)',
          {
            taskId,
            repositoryId: handle.repositoryId,
          },
        )
        try {
          await this.previewManager.start(taskId, repositoryId)
        } catch (error) {
          // Log error but don't fail workspace provisioning
          logger.warn('WorkspaceCoordinator: Failed to auto-start preview server', {
            taskId,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      } else {
        logger.debug('WorkspaceCoordinator: Preview server already started, skipping auto-start', {
          taskId,
        })
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
        await this.agentController.stopAgent(taskId)
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
}
