import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { LocalEnvironmentDriver } from '../../workspace/drivers/local-environment-driver'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { PreviewServerConfig } from '../types/config'
import { PreviewRuntimeService } from './preview-runtime.service'
import type { PreviewStateBridge } from './preview-state-bridge.interface'

/**
 * PreviewOrchestratorService - Orchestrates preview server lifecycle
 *
 * Responsibilities:
 * - Coordinate preview server start/stop/restart across features
 * - Fetch repository configuration and workspace state
 * - Update workspace preview state via PreviewStateBridge
 * - Delegate runtime execution to PreviewRuntimeService
 *
 * Key principle: Orchestrator coordinates, RuntimeService manages processes, Bridge updates state
 */
@injectable()
export class PreviewOrchestratorService {
  private readonly serviceLogger: typeof logger

  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(LocalEnvironmentDriver) private readonly driver: LocalEnvironmentDriver,
    @inject(PreviewRuntimeService) private readonly runtime: PreviewRuntimeService,
    @inject('PreviewStateBridge') private readonly stateBridge: PreviewStateBridge,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-orchestrator' })
  }

  /**
   * Start preview server for a task
   */
  async start(taskId: string, repositoryId?: string): Promise<ServerStatus> {
    this.serviceLogger.info('Starting preview server', { taskId })

    // 1. Ensure workspace and get enriched handle with runtime state
    const handle = await this.runtimeManager.ensureWorkspace(taskId, repositoryId)

    // 2. Update state to mark preview as starting (preserve lastStartedAt to detect first start)
    await this.stateBridge.updatePreviewState(taskId, {
      status: 'starting',
    })

    try {
      // 3. Fetch current repository configuration
      const repoConfig = await this.repoService.getRepositoryById(handle.repositoryId)
      if (!repoConfig) {
        throw new Error(`Repository not found: ${handle.repositoryId}`)
      }

      // 4. Execute setup commands if this is the first preview start
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
          handle.workspacePath,
          repoConfig.codeExecution.setupCommands,
          repoConfig.codeExecution.envVars,
        )
        this.serviceLogger.info(' Setup commands completed successfully', { taskId })
      }

      // 5. Build preview configuration
      const logFile = await this.driver.ensureTaskLog(taskId)

      const previewConfig: PreviewServerConfig = {
        serverId: taskId,
        workspacePath: handle.workspacePath,
        serverCommands: repoConfig.previewConfig?.serverStartCommands || [],
        envVars: repoConfig.codeExecution.envVars || undefined,
        logFile,
        enableInspector: true,
      }

      // 6. Delegate to runtime service to start preview server
      const status = await this.runtime.start(previewConfig)

      // 7. Update state based on server status
      if (status.status === 'started') {
        await this.stateBridge.updatePreviewState(taskId, {
          status: 'started',
          endpoint: status.url,
          proxyUrl: status.url,
          lastStartedAt: new Date(), // Set on successful start to track setup completion
        })
        this.serviceLogger.info('Preview server started', { taskId, url: status.url })
      } else {
        // Server failed to start
        await this.stateBridge.updatePreviewState(taskId, {
          status: 'failed',
          lastError: status.error || 'Preview server failed to start',
        })
        logger.error(
          'PreviewManager: Preview server failed to start',
          new Error(status.error || 'Unknown error'),
          { taskId },
        )
      }

      return status
    } catch (error) {
      // Update state with error
      await this.stateBridge.updatePreviewState(taskId, {
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
      await this.runtime.stop(taskId)

      // Update state
      await this.stateBridge.updatePreviewState(taskId, {
        status: 'stopped',
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
