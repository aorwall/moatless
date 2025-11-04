import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { LocalEnvironmentDriver } from '../../workspace/drivers/local-environment-driver'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { PreviewServerConfig } from '../types/config'
import { PreviewRuntimeService } from './preview-runtime.service'

/**
 * PreviewOrchestratorService - Orchestrates preview server lifecycle
 *
 * Responsibilities:
 * - Coordinate preview server start/stop/restart across features
 * - Fetch repository configuration and workspace state
 * - Update workspace preview state via WorkspaceRuntimeManager
 * - Delegate runtime execution to PreviewRuntimeService
 *
 * Key principle: Orchestrator coordinates, RuntimeService manages processes and state
 */
@injectable()
export class PreviewOrchestratorService {
  private readonly serviceLogger: typeof logger

  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(LocalEnvironmentDriver) private readonly driver: LocalEnvironmentDriver,
    @inject(PreviewRuntimeService) private readonly runtime: PreviewRuntimeService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-orchestrator' })
  }

  /**
   * Start preview server for a task
   */
  async start(taskId: string, repositoryId?: string): Promise<ServerStatus> {
    this.serviceLogger.info('Starting preview server', { taskId })

    // 1. Resolve repository ID if not provided
    let resolvedRepositoryId = repositoryId
    if (!resolvedRepositoryId) {
      const task = await this.taskRepository.get(taskId)
      if (task) {
        resolvedRepositoryId = task.repositoryId
        this.serviceLogger.debug('Resolved repository ID from task', {
          taskId,
          repositoryId: resolvedRepositoryId,
        })
      }
    }

    // 2. Ensure workspace and get enriched handle with runtime state
    const handle = await this.runtimeManager.ensureWorkspace(taskId, resolvedRepositoryId)

    // 3. Update state to mark preview as starting (preserve lastStartedAt to detect first start)
    await this.runtimeManager.updatePreviewState(taskId, {
      status: 'starting',
    })

    try {
      // 4. Fetch current repository configuration
      const repoConfig = await this.repoService.getRepositoryById(handle.repositoryId)
      if (!repoConfig) {
        throw new Error(`Repository not found: ${handle.repositoryId}`)
      }

      // 5. Execute setup commands if this is the first preview start
      if (
        handle.preview.needsSetup &&
        repoConfig.codeExecution?.setupCommands &&
        repoConfig.codeExecution.setupCommands.length > 0
      ) {
        this.serviceLogger.info(' Executing setup commands before first preview start', {
          taskId,
          commands: repoConfig.codeExecution.setupCommands,
        })
        await this.driver.executeSetupCommands(
          taskId,
          handle.workspacePath,
          repoConfig.codeExecution.setupCommands,
          repoConfig.codeExecution.envVars,
        )
        this.serviceLogger.info(' Setup commands completed successfully', { taskId })
      }

      // 6. Build preview configuration
      const logFile = await this.driver.ensureTaskLog(taskId)

      const previewConfig: PreviewServerConfig = {
        serverId: taskId,
        workspacePath: handle.workspacePath,
        serverCommands: repoConfig.previewConfig?.serverStartCommands || [],
        envVars: repoConfig.codeExecution.envVars || undefined,
        logFile,
        enableInspector: true,
      }

      // 7. Delegate to runtime service to start preview server
      // Note: PreviewRuntimeService now handles all state updates via lifecycle sink
      const status = await this.runtime.start(previewConfig)

      // 8. Update lastStartedAt on successful start to track setup completion
      if (status.status === 'started') {
        await this.runtimeManager.updatePreviewState(taskId, {
          lastStartedAt: new Date(),
        })
        this.serviceLogger.info('Preview server started', { taskId, url: status.url })
      } else {
        // Server failed to start
        logger.error(
          'PreviewManager: Preview server failed to start',
          new Error(status.error || 'Unknown error'),
          { taskId },
        )
      }

      return status
    } catch (error) {
      // Update state with error
      await this.runtimeManager.updatePreviewState(taskId, {
        status: 'failed',
        lastError: error instanceof Error ? error.message : String(error),
      })

      this.serviceLogger.error('Failed to start preview server', error as Error, { taskId })
      throw error
    }
  }

  /**
   * Stop preview server for a task
   */
  async stop(taskId: string): Promise<void> {
    this.serviceLogger.info('Stopping preview server', { taskId })

    try {
      // Delegate to runtime service to stop preview server
      // Note: PreviewRuntimeService handles state updates via lifecycle sink
      await this.runtime.stop(taskId)

      // Update lastStoppedAt timestamp
      await this.runtimeManager.updatePreviewState(taskId, {
        lastStoppedAt: new Date(),
      })

      this.serviceLogger.info('Preview server stopped', { taskId })
    } catch (error) {
      this.serviceLogger.error('Failed to stop preview server', error as Error, { taskId })
      throw error
    }
  }

  /**
   * Restart preview server for a task
   */
  async restart(taskId: string): Promise<ServerStatus> {
    this.serviceLogger.info(' Restarting preview server', { taskId })

    // Stop if running
    await this.stop(taskId)

    // Start fresh
    return await this.start(taskId)
  }

  /**
   * Get preview server status
   */
  async getStatus(taskId: string): Promise<ServerStatus> {
    return await this.runtime.getStatus(taskId)
  }

  /**
   * Get preview server logs
   */
  async getLogs(taskId: string, lines?: number): Promise<string> {
    return await this.runtime.getLogs(taskId, lines || 100)
  }

  /**
   * Stream preview server logs
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    yield* this.runtime.streamLogs(taskId)
  }

  /**
   * Get preview port
   */
  getPort(taskId: string): number | undefined {
    return this.runtime.getPort(taskId)
  }

  /**
   * Cleanup - stop all active servers
   */
  async cleanup(): Promise<void> {
    this.serviceLogger.info('Cleaning up all preview servers')
    await this.runtime.cleanup()
  }
}
