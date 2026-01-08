import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import type {
  BaseEnvironmentDriver,
  PreviewServerConfig,
} from '../../workspace/drivers/base-environment-driver'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { WorkspaceHandle } from '../../workspace/types/workspace-handle.types'
import { loadProxyConfig, type ProxyEnvironmentConfig } from '../config/proxy-config'
import { PlaywrightInteractionService } from './playwright-interaction.service'
import { PreviewRuntimeService } from './preview-runtime.service'

/**
 * PreviewOrchestratorService - Orchestrates preview server lifecycle
 *
 * Responsibilities:
 * - Coordinate preview server start/stop/restart across features
 * - Fetch repository configuration and workspace state
 * - Update workspace preview state via WorkspaceRuntimeManager
 * - Delegate runtime execution to environment drivers
 * - Wire up Playwright service with lifecycle sink
 *
 * Key principle: Orchestrator coordinates, RuntimeService manages processes and state
 */
@injectable()
export class PreviewOrchestratorService {
  private readonly serviceLogger: typeof logger
  private readonly proxyConfig: ProxyEnvironmentConfig

  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(PreviewRuntimeService) public readonly runtime: PreviewRuntimeService,
    @inject(PlaywrightInteractionService)
    private readonly playwrightService: PlaywrightInteractionService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-orchestrator' })
    this.proxyConfig = loadProxyConfig()

    // Wire up Playwright service with PreviewRuntimeService as lifecycle sink
    // This enables Playwright actions to emit SSE events
    this.playwrightService.setLifecycleSink(this.runtime)

    // Wire up Playwright service with this orchestrator for server status checks
    // This enables Playwright operations to verify server is running
    this.playwrightService.setPreviewOrchestrator(this)

    // Wire up Playwright service with repository service
    // This enables Playwright operations to fetch repository config
    this.playwrightService.setRepositoryService(this.repoService)

    this.serviceLogger.debug(
      {
        lifecycleSink: 'PreviewRuntimeService',
        repositoryService: 'RepositoryService',
      },
      'Playwright services configured',
    )
  }

  private async getWorkspaceHandleIfExists(taskId: string): Promise<WorkspaceHandle | null> {
    return await this.runtimeManager.getExistingWorkspaceHandle(taskId)
  }

  /**
   * Start preview server for a task
   * Starts the server in the background and returns immediately
   * Status updates are emitted via SSE events
   */
  async start(taskId: string, repositoryId?: string): Promise<void> {
    this.serviceLogger.info({ taskId }, 'Starting preview server')

    // 1. Resolve repository ID if not provided
    let resolvedRepositoryId = repositoryId
    if (!resolvedRepositoryId) {
      const task = await this.taskRepository.get(taskId)
      if (task) {
        resolvedRepositoryId = task.repositoryId
        this.serviceLogger.debug(
          {
            taskId,
            repositoryId: resolvedRepositoryId,
          },
          'Resolved repository ID from task',
        )
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
      serverCommands: repoConfig.previewConfig.serverStartCommands,
      enableInspector: true,
      ...(handle.preview.needsSetup &&
        repoConfig.codeExecution?.setupCommands && {
          setupCommands: repoConfig.codeExecution.setupCommands,
        }),
      ...(repoConfig.codeExecution?.envVars && { envVars: repoConfig.codeExecution.envVars }),
      ...(repoConfig.previewConfig?.startupTimeoutSeconds && {
        startupTimeoutSeconds: repoConfig.previewConfig.startupTimeoutSeconds,
      }),
    }

    this.serviceLogger.debug({ taskId }, 'Starting server in background')

    // Update state to mark preview as starting
    await this.runtimeManager.updatePreviewState(taskId, { status: 'starting' })

    // Start server in background (don't await)
    // The runtime service or driver will emit events via SSE as status changes
    this.serviceLogger.debug({ taskId }, 'Initiating background start')
    this.startBackground(taskId, handle.driver, previewConfig).catch((error) => {
      this.serviceLogger.error({ err: error as Error, taskId }, 'Background start failed')
    })
  }

  /**
   * Background execution of preview server start
   * Starts the server, emitting events along the way
   * Setup commands are now handled by PreviewServer.start() as part of the command sequence
   */
  private async startBackground(
    taskId: string,
    driver: BaseEnvironmentDriver,
    previewConfig: PreviewServerConfig,
  ): Promise<void> {
    try {
      if (!driver.startPreviewServer) {
        throw new Error('Preview server not supported by environment driver')
      }

      const result = await driver.startPreviewServer(previewConfig)
      if (result.status === 'failed') {
        throw new Error(result.error || 'Preview server failed to start')
      }

      this.serviceLogger.info({ taskId }, 'Preview server start completed')
    } catch (error) {
      this.serviceLogger.error(
        { err: error as Error, taskId },
        'Failed to start preview server in background',
      )
      await this.runtimeManager.updatePreviewState(taskId, {
        status: 'failed',
        lastError: error instanceof Error ? error.message : String(error),
      })
      throw error
    }
  }

  /**
   * Stop preview server for a task
   */
  async stop(taskId: string): Promise<void> {
    this.serviceLogger.debug({ taskId }, 'Stopping preview server')

    try {
      const handle = await this.getWorkspaceHandleIfExists(taskId)
      if (!handle?.driver.stopPreviewServer) {
        return
      }

      await handle.driver.stopPreviewServer()

      this.serviceLogger.debug({ taskId }, 'Preview server stopped')
    } catch (error) {
      this.serviceLogger.error({ err: error as Error, taskId }, 'Failed to stop preview server')
      throw error
    }
  }

  /**
   * Restart preview server for a task
   */
  async restart(taskId: string): Promise<void> {
    this.serviceLogger.info({ taskId }, 'Restarting preview server')

    // Stop if running
    await this.stop(taskId)

    // Start fresh
    await this.start(taskId)
  }

  /**
   * Start or restart preview server
   * Decides whether to restart based on dirty context and auto-reload configuration
   * @param taskId
   * @param repositoryId
   * @param force If true, always restart regardless of dirty context or auto-reload
   * @returns Server status and optional skip message
   */
  async startOrRestart(
    taskId: string,
    repositoryId?: string,
    force = false,
  ): Promise<{ status: ServerStatus; skipMessage?: string }> {
    this.serviceLogger.info(
      {
        taskId,
        force,
      },
      'Starting or restarting preview server',
    )

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

    this.serviceLogger.info(
      {
        taskId,
        isRunning,
        isDirty,
        autoReload,
        force,
      },
      'Preview server restart decision',
    )

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
      this.serviceLogger.info({ taskId }, 'Force restarting preview server (explicit force flag)')
      await this.stop(taskId)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      return undefined
    }

    if (isDirty && !autoReload) {
      // Code changes detected and no auto-reload - full restart needed
      this.serviceLogger.info(
        {
          taskId,
        },
        'Restarting preview server due to code changes (no auto-reload)',
      )
      await this.stop(taskId)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      return undefined
    }

    if (isDirty && autoReload) {
      // Code changes but auto-reload enabled - skip restart
      this.serviceLogger.info(
        { taskId },
        'Skipping preview server restart - auto-reload enabled (changes will be picked up automatically)',
      )
      return `Preview server is already running with auto-reload enabled. Code changes will be picked up automatically. Status: ${currentStatus.status}${currentStatus.url ? `, URL: ${currentStatus.url}` : ''}`
    }

    // Server running and no code changes - restart anyway (explicit request)
    this.serviceLogger.info({ taskId }, 'Restarting preview server (explicit request)')
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
    this.serviceLogger.info({ taskId }, 'Starting preview server and waiting for ready')

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

    this.serviceLogger.info(
      {
        taskId,
        timeoutSeconds,
        pollIntervalMs,
      },
      'Starting server with timeout',
    )

    // 3. Start the server (background operation)
    await this.start(taskId, resolvedRepositoryId)

    // 4. Poll for status until ready or timeout
    const startTime = Date.now()
    let lastStatus: ServerStatus | undefined

    while (Date.now() - startTime < timeoutMs) {
      const status = await this.getStatus(taskId)
      lastStatus = status

      this.serviceLogger.debug(
        {
          taskId,
          status: status.status,
          elapsedMs: Date.now() - startTime,
        },
        'Polling server status',
      )

      // Success - server is started
      if (status.status === 'started') {
        this.serviceLogger.info(
          {
            taskId,
            url: status.url,
            elapsedMs: Date.now() - startTime,
          },
          'Preview server ready',
        )
        return status
      }

      // Cancellation - server was stopped or task deleted
      if (status.status === 'stopped') {
        this.serviceLogger.info(
          {
            taskId,
            status: status.status,
          },
          'Server was stopped during startup wait',
        )
        // Return current status instead of throwing - this is expected during cleanup
        return status
      }

      // Failure - server failed to start
      if (status.status === 'failed') {
        const errorMsg = status.error || 'Unknown error during server startup'
        this.serviceLogger.error(
          { err: new Error(errorMsg), taskId, elapsedMs: Date.now() - startTime },
          'Preview server failed to start',
        )
        throw new Error(`Preview server failed to start: ${errorMsg}`)
      }

      // Still starting - wait before next poll
      await new Promise((resolve) => setTimeout(resolve, pollIntervalMs))
    }

    // Timeout - server didn't reach 'started' status in time
    const errorMsg = `Preview server did not start within ${timeoutSeconds} seconds. Last status: ${lastStatus?.status || 'unknown'}`
    this.serviceLogger.error(
      { err: new Error(errorMsg), taskId, timeoutSeconds, lastStatus: lastStatus?.status },
      'Preview server startup timeout',
    )
    throw new Error(errorMsg)
  }

  /**
   * Get preview server status
   */
  async getStatus(taskId: string): Promise<ServerStatus> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getPreviewServerStatus) {
      return {
        status: 'stopped',
        pid: undefined,
        url: undefined,
        uptimeSeconds: 0,
        dirtyContext: snapshot?.preview?.dirtyContext,
      }
    }

    const status = await handle.driver.getPreviewServerStatus()
    const externalUrl =
      status.externalUrl ||
      (status.status === 'started' &&
      this.proxyConfig.useHostnameRouting &&
      this.proxyConfig.proxyDomain
        ? `${this.proxyConfig.proxyProtocol}://${taskId}.${this.proxyConfig.proxyDomain}`
        : undefined)

    return {
      status: status.status,
      pid: status.pid,
      url: externalUrl,
      uptimeSeconds: status.uptimeSeconds || 0,
      error: status.error,
      logs: status.logs,
      dirtyContext: snapshot?.preview?.dirtyContext,
    }
  }

  /**
   * Get internal server URL for backend services (Playwright, etc.)
   * This bypasses authentication - use only for internal tool/agent access
   */
  async getInternalUrl(taskId: string): Promise<string | undefined> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getPreviewServerStatus) {
      return undefined
    }

    const status = await handle.driver.getPreviewServerStatus()
    if (status.status !== 'started') {
      return undefined
    }
    return status.internalUrl
  }

  /**
   * Get preview server logs
   */
  async getLogs(taskId: string, lines?: number): Promise<string> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getPreviewServerLogs) {
      return 'No preview server running'
    }

    return await handle.driver.getPreviewServerLogs(lines || 100)
  }

  /**
   * Stream preview server logs
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.streamPreviewServerLogs) {
      yield {
        type: 'error',
        data: {
          error: 'Preview server is not running',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    yield* handle.driver.streamPreviewServerLogs(1000)
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
