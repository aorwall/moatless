import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { getTaskLogPath } from '../../../utils/task-log'
import { RepositoryService } from '../../repositories/index'
import type { PreviewConfiguration } from '../drivers/base-environment-driver'
import { LocalEnvironmentDriver } from '../drivers/local-environment-driver'
import { WorkspaceRuntimeStorage } from '../storage/workspace-runtime.storage'
import { WorkspaceCoordinator } from './workspace-coordinator.service'

/**
 * PreviewManager - Manages preview server lifecycle
 *
 * Responsibilities:
 * - Orchestrate preview server start/stop/restart
 * - Update runtime snapshots with preview state
 * - Delegate actual execution to environment drivers
 *
 * Key principle: Manager orchestrates and updates snapshots, driver executes operations
 */
@injectable()
export class PreviewManager {
  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceCoordinator) private readonly coordinator: WorkspaceCoordinator,
    @inject(WorkspaceRuntimeStorage) private readonly runtimeStore: WorkspaceRuntimeStorage,
    @inject(LocalEnvironmentDriver) private readonly driver: LocalEnvironmentDriver,
  ) {}

  /**
   * Start preview server for a task
   */
  async start(taskId: string, repositoryId?: string): Promise<ServerStatus> {
    logger.info('PreviewManager: Starting preview server', { taskId })

    // 1. Load snapshot to get workspace path
    const handle = await this.coordinator.ensureWorkspace(taskId, repositoryId)
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      throw new Error(`No workspace snapshot found for task: ${taskId}`)
    }

    // 2. Update snapshot to mark preview as starting (don't set lastStartedAt yet)
    await this.runtimeStore.update(taskId, (s) => ({
      ...s,
      preview: {
        status: 'starting',
        lastStartedAt: s.preview.lastStartedAt, // Preserve existing value to detect first start
      },
    }))

    try {
      // 3. Fetch current repository configuration
      const repoConfig = await this.repoService.getRepositoryById(snapshot.repositoryId)
      if (!repoConfig) {
        throw new Error(`Repository not found: ${snapshot.repositoryId}`)
      }

      // 4. Execute setup commands if this is the first preview start
      const needsSetup = !snapshot.preview.lastStartedAt
      if (
        needsSetup &&
        repoConfig.codeExecution?.setupCommands &&
        repoConfig.codeExecution.setupCommands.length > 0
      ) {
        logger.info('PreviewManager: Executing setup commands before first preview start', {
          taskId,
          commands: repoConfig.codeExecution.setupCommands,
        })
        await this.driver.executeSetupCommands(
          handle.workspacePath,
          repoConfig.codeExecution.setupCommands,
          repoConfig.codeExecution.envVars,
        )
        logger.info('PreviewManager: Setup commands completed successfully', { taskId })
      }

      // 5. Build preview configuration
      const workspacePath = handle.workspacePath
      const logFile = getTaskLogPath(taskId)

      const previewConfig: PreviewConfiguration = {
        taskId,
        workspacePath,
        serverStartCommands: repoConfig.previewConfig?.serverStartCommands || [],
        envVars: repoConfig.codeExecution.envVars || undefined,
        logFile,
      }

      // 6. Delegate to driver to start preview server
      const status = await this.driver.startPreview(previewConfig)

      // 7. Update snapshot based on server status
      if (status.status === 'started') {
        await this.runtimeStore.update(taskId, (s) => ({
          ...s,
          preview: {
            status: 'started',
            endpoint: status.url,
            proxyUrl: status.url,
            lastStartedAt: new Date(), // Set on successful start to track setup completion
          },
        }))
        logger.info('PreviewManager: Preview server started', { taskId, url: status.url })
      } else {
        // Server failed to start
        await this.runtimeStore.update(taskId, (s) => ({
          ...s,
          preview: {
            status: 'failed',
            lastError: status.error || 'Preview server failed to start',
            lastStartedAt: s.preview.lastStartedAt, // Preserve on failure
          },
        }))
        logger.error(
          'PreviewManager: Preview server failed to start',
          new Error(status.error || 'Unknown error'),
          { taskId },
        )
      }

      return status
    } catch (error) {
      // Update snapshot with error
      await this.runtimeStore.update(taskId, (s) => ({
        ...s,
        preview: {
          status: 'failed',
          lastError: error instanceof Error ? error.message : String(error),
          lastStartedAt: s.preview.lastStartedAt,
        },
      }))

      logger.error('PreviewManager: Failed to start preview server', error as Error, { taskId })
      throw error
    }
  }

  /**
   * Stop preview server for a task
   */
  async stop(taskId: string): Promise<void> {
    logger.info('PreviewManager: Stopping preview server', { taskId })

    try {
      // Delegate to driver to stop preview server
      await this.driver.stopPreview(taskId)

      // Update snapshot
      await this.runtimeStore.update(taskId, (s) => ({
        ...s,
        preview: {
          status: 'stopped',
          lastStoppedAt: new Date(),
        },
      }))

      logger.info('PreviewManager: Preview server stopped', { taskId })
    } catch (error) {
      logger.error('PreviewManager: Failed to stop preview server', error as Error, { taskId })
      throw error
    }
  }

  /**
   * Restart preview server for a task
   */
  async restart(taskId: string): Promise<ServerStatus> {
    logger.info('PreviewManager: Restarting preview server', { taskId })

    // Stop if running
    await this.stop(taskId)

    // Start fresh
    return await this.start(taskId)
  }

  /**
   * Get preview server status
   */
  async getStatus(taskId: string): Promise<ServerStatus> {
    return await this.driver.getPreviewStatus(taskId)
  }

  /**
   * Get preview server logs
   */
  async getLogs(taskId: string, lines?: number): Promise<string> {
    return await this.driver.getPreviewLogs(taskId, lines)
  }

  /**
   * Stream preview server logs
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    yield* this.driver.streamPreviewLogs(taskId)
  }

  /**
   * Proxy request to preview server
   */
  async proxyRequest(
    taskId: string,
    request: {
      method: string
      path: string
      headers: Headers
      body?: string
    },
  ): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }> {
    return await this.driver.proxyPreviewRequest(taskId, request)
  }

  /**
   * Get preview port
   */
  getPort(taskId: string): number | undefined {
    return this.driver.getPreviewPort(taskId)
  }

  /**
   * Cleanup - stop all active servers
   */
  async cleanup(): Promise<void> {
    logger.info('PreviewManager: Cleaning up all preview servers')
    await this.driver.cleanup()
  }
}
