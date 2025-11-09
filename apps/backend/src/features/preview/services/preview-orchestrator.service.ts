import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { PreviewServerConfig } from '../types/config'
import { PlaywrightInteractionService } from './playwright-interaction.service'
import { PreviewRuntimeService } from './preview-runtime.service'

/**
 * PreviewOrchestratorService - Orchestrates preview server lifecycle
 *
 * Responsibilities:
 * - Coordinate preview server start/stop/restart across features
 * - Fetch repository configuration and workspace state
 * - Update workspace preview state via WorkspaceRuntimeManager
 * - Delegate runtime execution to PreviewRuntimeService
 * - Wire up Playwright service with lifecycle sink
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
    @inject(PreviewRuntimeService) public readonly runtime: PreviewRuntimeService,
    @inject(PlaywrightInteractionService)
    private readonly playwrightService: PlaywrightInteractionService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-orchestrator' })

    // Wire up Playwright service with PreviewRuntimeService as lifecycle sink
    // This enables Playwright actions to emit SSE events
    this.playwrightService.setLifecycleSink(this.runtime)

    // Wire up Playwright service with this orchestrator for server status checks
    // This enables Playwright operations to verify server is running
    this.playwrightService.setPreviewOrchestrator(this)

    // Wire up Playwright service with repository service
    // This enables Playwright operations to fetch repository config
    this.playwrightService.setRepositoryService(this.repoService)

    this.serviceLogger.debug('Playwright services configured', {
      lifecycleSink: 'PreviewRuntimeService',
      repositoryService: 'RepositoryService',
    })
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
    // Validate that we have server start commands
    if (
      !repoConfig.previewConfig?.serverStartCommands ||
      repoConfig.previewConfig.serverStartCommands.length === 0
    ) {
      throw new Error(
        `Preview server cannot start: No server start commands configured for repository '${repoConfig.name}'. ` +
          'Please configure serverStartCommands in the repository preview settings.',
      )
    }

    const previewConfig: PreviewServerConfig = {
      serverId: taskId,
      workspacePath: handle.workspacePath,
      serverCommands: repoConfig.previewConfig.serverStartCommands,
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
   * Start or restart preview server with smart logic
   * Decides whether to restart based on dirty context and auto-reload configuration
   * @param force If true, always restart regardless of dirty context or auto-reload
   * @returns Server status and optional skip message
   */
  async startOrRestart(
    taskId: string,
    repositoryId?: string,
    force = false,
  ): Promise<{ status: ServerStatus; skipMessage?: string }> {
    this.serviceLogger.info('Starting or restarting preview server with smart logic', {
      taskId,
      force,
    })

    // 1. Resolve repository ID if not provided
    let resolvedRepositoryId = repositoryId
    if (!resolvedRepositoryId) {
      const task = await this.taskRepository.get(taskId)
      if (task) {
        resolvedRepositoryId = task.repositoryId
      }
    }

    if (!resolvedRepositoryId) {
      throw new Error('Repository ID not found for task')
    }

    // 2. Get repository config to check auto-reload
    const repoConfig = await this.repoService.getRepositoryById(resolvedRepositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${resolvedRepositoryId}`)
    }

    const autoReload = repoConfig.previewConfig?.autoReload ?? false

    // 3. Check if server is running and dirty
    const currentStatus = await this.getStatus(taskId)
    const isRunning = currentStatus.status === 'started' || currentStatus.status === 'starting'
    const isDirty = currentStatus.dirtyContext ?? false

    this.serviceLogger.info('Preview server restart decision', {
      taskId,
      isRunning,
      isDirty,
      autoReload,
      force,
    })

    // 4. Handle restart decision if server is running
    if (isRunning) {
      const skipMessage = await this.handleRunningServerRestart(
        taskId,
        currentStatus,
        isDirty,
        autoReload,
        force,
      )
      if (skipMessage) {
        return { status: currentStatus, skipMessage }
      }
    }

    // 5. Start the server and wait for it to be ready
    const serverStatus = await this.startAndWaitForReady(taskId, resolvedRepositoryId)

    return { status: serverStatus }
  }

  /**
   * Handle restart decision for running server
   * Returns skip message if restart should be skipped, undefined otherwise
   */
  private async handleRunningServerRestart(
    taskId: string,
    currentStatus: ServerStatus,
    isDirty: boolean,
    autoReload: boolean,
    force: boolean,
  ): Promise<string | undefined> {
    if (force) {
      // Force restart - skip all smart logic
      this.serviceLogger.info('Force restarting preview server (explicit force flag)', { taskId })
      await this.stop(taskId)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      return undefined
    }

    if (isDirty && !autoReload) {
      // Code changes detected and no auto-reload - full restart needed
      this.serviceLogger.info('Restarting preview server due to code changes (no auto-reload)', {
        taskId,
      })
      await this.stop(taskId)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      return undefined
    }

    if (isDirty && autoReload) {
      // Code changes but auto-reload enabled - skip restart
      this.serviceLogger.info(
        'Skipping preview server restart - auto-reload enabled (changes will be picked up automatically)',
        { taskId },
      )
      return `Preview server is already running with auto-reload enabled. Code changes will be picked up automatically. Status: ${currentStatus.status}${currentStatus.url ? `, URL: ${currentStatus.url}` : ''}`
    }

    // Server running and no code changes - restart anyway (explicit request)
    this.serviceLogger.info('Restarting preview server (explicit request)', { taskId })
    await this.stop(taskId)
    await new Promise((resolve) => setTimeout(resolve, 1000))
    return undefined
  }

  /**
   * Start preview server and wait for it to be ready
   * Uses repository's configured startup timeout (default: 30s)
   * @throws Error if server fails to start or times out
   */
  async startAndWaitForReady(taskId: string, repositoryId?: string): Promise<ServerStatus> {
    this.serviceLogger.info('Starting preview server and waiting for ready', { taskId })

    // 1. Resolve repository ID if not provided
    let resolvedRepositoryId = repositoryId
    if (!resolvedRepositoryId) {
      const task = await this.taskRepository.get(taskId)
      if (task) {
        resolvedRepositoryId = task.repositoryId
      }
    }

    if (!resolvedRepositoryId) {
      throw new Error('Repository ID not found for task')
    }

    // 2. Get repository config to determine timeout
    const repoConfig = await this.repoService.getRepositoryById(resolvedRepositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${resolvedRepositoryId}`)
    }

    // Get timeout from repository config (default: 30 seconds)
    const timeoutSeconds = repoConfig.previewConfig?.startupTimeoutSeconds ?? 30
    const timeoutMs = timeoutSeconds * 1000
    const pollIntervalMs = 500

    this.serviceLogger.info('Starting server with timeout', {
      taskId,
      timeoutSeconds,
      pollIntervalMs,
    })

    // 3. Start the server (background operation)
    await this.start(taskId, resolvedRepositoryId)

    // 4. Poll for status until ready or timeout
    const startTime = Date.now()
    let lastStatus: ServerStatus | undefined

    while (Date.now() - startTime < timeoutMs) {
      const status = await this.getStatus(taskId)
      lastStatus = status

      this.serviceLogger.debug('Polling server status', {
        taskId,
        status: status.status,
        elapsedMs: Date.now() - startTime,
      })

      // Success - server is started
      if (status.status === 'started') {
        this.serviceLogger.info('Preview server ready', {
          taskId,
          url: status.url,
          elapsedMs: Date.now() - startTime,
        })
        return status
      }

      // Failure - server failed to start
      if (status.status === 'failed') {
        const errorMsg = status.error || 'Unknown error during server startup'
        this.serviceLogger.error('Preview server failed to start', new Error(errorMsg), {
          taskId,
          elapsedMs: Date.now() - startTime,
        })
        throw new Error(`Preview server failed to start: ${errorMsg}`)
      }

      // Still starting - wait before next poll
      await new Promise((resolve) => setTimeout(resolve, pollIntervalMs))
    }

    // Timeout - server didn't reach 'started' status in time
    const errorMsg = `Preview server did not start within ${timeoutSeconds} seconds. Last status: ${lastStatus?.status || 'unknown'}`
    this.serviceLogger.error('Preview server startup timeout', new Error(errorMsg), {
      taskId,
      timeoutSeconds,
      lastStatus: lastStatus?.status,
    })
    throw new Error(errorMsg)
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
