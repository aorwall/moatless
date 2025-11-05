import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
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
    @inject(PreviewRuntimeService) private readonly runtime: PreviewRuntimeService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-orchestrator' })
  }

  /**
   * Start preview server for a task
   * Starts the server in the background and returns immediately
   * Status updates are emitted via SSE events
   */
  async start(taskId: string, repositoryId?: string): Promise<void> {
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

    // 3. Fetch current repository configuration
    const repoConfig = await this.repoService.getRepositoryById(handle.repositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${handle.repositoryId}`)
    }

    // 4. Build preview configuration
    const previewConfig: PreviewServerConfig = {
      serverId: taskId,
      workspacePath: handle.workspacePath,
      serverCommands: repoConfig.previewConfig?.serverStartCommands || [],
      setupCommands: handle.preview.needsSetup
        ? repoConfig.codeExecution?.setupCommands
        : undefined,
      envVars: repoConfig.codeExecution.envVars || undefined,
      enableInspector: true,
    }

    this.serviceLogger.info('Starting server in background', { taskId })

    // 5. Update state to mark preview as starting
    await this.runtimeManager.updatePreviewState(taskId, {
      status: 'starting',
    })

    // 6. Start server in background (don't await)
    // The runtime service will emit events via SSE as status changes
    this.serviceLogger.info('Initiating background start', { taskId })
    this.startBackground(taskId, previewConfig).catch((error) => {
      this.serviceLogger.error('Background start failed', error as Error, { taskId })
    })
  }

  /**
   * Background execution of preview server start
   * Starts the server, emitting events along the way
   * Setup commands are now handled by PreviewServer.start() as part of the command sequence
   */
  private async startBackground(taskId: string, previewConfig: PreviewServerConfig): Promise<void> {
    try {
      // Delegate to runtime service to start preview server
      // PreviewServer.start() will execute setupCommands (if provided) followed by server start
      // Note: PreviewRuntimeService handles all state updates via lifecycle sink
      await this.runtime.start(previewConfig)

      this.serviceLogger.info('Preview server start completed', { taskId })
    } catch (error) {
      // Update state with error
      await this.runtimeManager.updatePreviewState(taskId, {
        status: 'failed',
        lastError: error instanceof Error ? error.message : String(error),
      })

      this.serviceLogger.error('Failed to start preview server in background', error as Error, {
        taskId,
      })
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
  async restart(taskId: string): Promise<void> {
    this.serviceLogger.info('Restarting preview server', { taskId })

    // Stop if running
    await this.stop(taskId)

    // Start fresh
    await this.start(taskId)
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
