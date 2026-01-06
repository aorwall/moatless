import { randomUUID } from 'node:crypto'
import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { activePreviewServers, previewServerTotal } from '../../metrics'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { PreviewState } from '../../workspace/types/workspace-runtime.types'
import { PreviewServer } from '../runtime/preview-server'
import type {
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerStatus,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
} from '../types/index.js'
import { isProcessAlive } from '../utils/process.utils'

/**
 * Convert PreviewServerStatus → ServerStatus
 * Maps preview server status to moatless-specific status
 */
function toServerStatus(status: PreviewServerStatus): ServerStatus {
  return {
    status: status.status,
    pid: status.pid,
    url: status.externalUrl, // Expose only external URL to API consumers
    uptimeSeconds: status.uptimeSeconds || 0,
    currentPath: status.currentPath,
    error: status.error,
    logs: status.logs,
    // internalUrl is not exposed in the public API (internal use only)
  }
}

/**
 * PreviewRuntimeService - Manages active preview server instances
 *
 * Responsibilities:
 * - Maintain map of taskId → PreviewServer instances
 * - Start/stop/restart preview servers
 * - Stream logs from servers
 * - Cleanup on shutdown
 * - Update preview state in workspace snapshots (implements PreviewServerLifecycleSink)
 *
 * Design:
 * - Uses WorkspaceRuntimeManager as single source of truth for state
 * - Creates PreviewServer instances and acts as their lifecycle sink
 * - Delegates process management to PreviewServer
 */
@injectable()
export class PreviewRuntimeService implements PreviewServerLifecycleSink {
  private readonly servers = new Map<string, PreviewServer>()
  private readonly serviceLogger: typeof logger
  private readonly eventUnsubscribe?: () => void

  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(EventService) private readonly eventService: EventService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-runtime' })

    // Subscribe to agent events for dirty context tracking
    this.eventUnsubscribe = this.eventService.subscribeAll((event) => {
      this.handleAgentEvent(event).catch((error) => {
        this.serviceLogger.error(
          {
            err: error as Error,
            eventType: event.type,
            eventSubType: 'subType' in event ? event.subType : undefined,
          },
          'Error handling agent event for dirty tracking',
        )
      })
    })

    this.serviceLogger.debug('Subscribed to agent events for dirty context tracking')
  }

  /**
   * PreviewServerLifecycleSink implementation - register server snapshot
   */
  async register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        status: snapshot.status,
      },
      'Registering preview server state',
    )
    await this.runtimeManager.updatePreviewState(serverId, {
      status: snapshot.status,
      port: snapshot.port,
      endpoint: snapshot.internalUrl,
      proxyUrl: snapshot.externalUrl,
      pid: snapshot.pid,
      inspectorEnabled: snapshot.inspectorEnabled,
      lastError: snapshot.error,
    })
  }

  /**
   * PreviewServerLifecycleSink implementation - update server state
   */
  async update(serverId: string, changes: PreviewServerUpdate): Promise<void> {
    this.serviceLogger.debug({ serverId, changes }, 'Updating preview server state')

    // Build update object with only defined properties (exactOptionalPropertyTypes)
    const updates: Partial<PreviewState> = {}
    if (changes.status !== undefined) {
      updates.status = changes.status
      // Set lastStartedAt when status changes to 'started'
      if (changes.status === 'started') {
        updates.lastStartedAt = new Date()
      }
    }
    if (changes.port !== undefined) {
      updates.port = changes.port
    }
    if (changes.internalUrl !== undefined) {
      updates.endpoint = changes.internalUrl
    }
    if (changes.externalUrl !== undefined) {
      updates.proxyUrl = changes.externalUrl
    }
    if (changes.pid !== undefined) {
      updates.pid = changes.pid
    }
    if (changes.inspectorEnabled !== undefined) {
      updates.inspectorEnabled = changes.inspectorEnabled
    }
    if (changes.error !== undefined) {
      updates.lastError = changes.error
    }

    await this.runtimeManager.updatePreviewState(serverId, updates)
  }

  /**
   * PreviewServerLifecycleSink implementation - remove server state
   */
  async remove(serverId: string): Promise<void> {
    this.serviceLogger.debug({ serverId }, 'Removing preview server state')
    // Mark as stopped when removed
    await this.runtimeManager.updatePreviewState(serverId, {
      status: 'stopped',
      port: undefined,
      endpoint: undefined,
      proxyUrl: undefined,
      pid: undefined,
      lastStoppedAt: new Date(),
    })
  }

  /**
   * PreviewServerLifecycleSink implementation - handle status changes
   * Emits ServerEvent to EventService for SSE streaming
   */
  async onStatusChange?(serverId: string, statusInfo: ServerStatusChangeInfo): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        status: statusInfo.status,
      },
      'Preview server status changed, emitting event',
    )

    try {
      // Map status to event subType
      const subType = statusInfo.status as 'starting' | 'started' | 'stopped' | 'failed'

      // Emit ServerEvent
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'server',
        subType,
        taskId: serverId,
        timestamp: new Date(),
        isError: statusInfo.status === 'failed',
        payload: {
          status: statusInfo.status,
          port: statusInfo.port,
          pid: statusInfo.pid,
          url: statusInfo.externalUrl, // Frontend gets external URL (proxy URL)
          error: statusInfo.error,
          uptimeSeconds: statusInfo.uptimeSeconds,
        },
      })

      this.serviceLogger.debug(
        {
          serverId,
          status: statusInfo.status,
        },
        'ServerEvent emitted successfully',
      )
    } catch (error) {
      // Log error but don't throw - event emission should not break server lifecycle
      this.serviceLogger.error(
        {
          err: error as Error,
          serverId,
          status: statusInfo.status,
        },
        'Failed to emit ServerEvent',
      )
    }
  }

  /**
   * PreviewServerLifecycleSink implementation - handle Playwright actions
   * Emits PreviewEvent with playwright-action subType for SSE streaming
   */
  async onPlaywrightAction?(
    serverId: string,
    actionInfo: import('../types/index.js').PlaywrightActionInfo,
  ): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        type: actionInfo.type,
        url: actionInfo.url,
        mode: actionInfo.mode,
      },
      'Playwright action occurred, emitting event',
    )

    try {
      // Emit PreviewEvent for Playwright action
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'preview',
        subType: 'playwright-action',
        taskId: serverId,
        timestamp: new Date(),
        isError: false,
        payload: {
          actionType: actionInfo.type as
            | 'navigate'
            | 'click'
            | 'evaluate'
            | 'fillForm'
            | 'snapshot',
          url: actionInfo.url,
          mode: actionInfo.mode,
          timestamp: actionInfo.timestamp,
        },
      })

      this.serviceLogger.debug(
        {
          serverId,
          actionType: actionInfo.type,
        },
        'PlaywrightAction event emitted successfully',
      )
    } catch (error) {
      // Log error but don't throw - event emission should not break Playwright actions
      this.serviceLogger.error(
        {
          err: error as Error,
          serverId,
          actionType: actionInfo.type,
        },
        'Failed to emit PlaywrightAction event',
      )
    }
  }

  /**
   * PreviewServerLifecycleSink implementation - handle Playwright mode changes
   * Emits PreviewEvent with mode-change subType for SSE streaming
   */
  async onPlaywrightModeChange?(serverId: string, mode: 'idle' | 'user' | 'agent'): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        mode,
      },
      'Playwright mode changed, emitting event',
    )

    try {
      // Emit PreviewEvent for mode change
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'preview',
        subType: 'mode-change',
        taskId: serverId,
        timestamp: new Date(),
        isError: false,
        payload: {
          mode,
        },
      })

      this.serviceLogger.debug(
        {
          serverId,
          mode,
        },
        'PlaywrightModeChange event emitted successfully',
      )
    } catch (error) {
      // Log error but don't throw - event emission should not break mode changes
      this.serviceLogger.error(
        {
          err: error as Error,
          serverId,
          mode,
        },
        'Failed to emit PlaywrightModeChange event',
      )
    }
  }

  /**
   * Handle agent events to detect code changes
   * Listens for tool_use events that modify the workspace
   */
  private async handleAgentEvent(event: import('@moatless/types').Event): Promise<void> {
    // Only process agent message events for dirty tracking
    if (event.type !== 'agent' || event.subType !== 'message') {
      return
    }

    // Detect code-changing tool use
    await this.detectCodeChangingTools(event)
  }

  /**
   * Detect code-changing tools in agent message events
   */
  private async detectCodeChangingTools(event: import('@moatless/types').Event): Promise<void> {
    const taskId = event.taskId

    // Check if we have a server for this task
    const server = this.servers.get(taskId)
    if (!server) {
      return // No server to mark dirty
    }

    // Parse SDK message for tool_use blocks
    const payload = event.payload as unknown
    if (!(payload && typeof payload === 'object' && 'message' in payload)) {
      return
    }

    const message = payload.message
    if (!(message && typeof message === 'object' && 'content' in message)) {
      return
    }

    const content = Array.isArray(message.content) ? message.content : [message.content]

    // Look for code-changing tool use
    for (const contentItem of content) {
      if (!contentItem || typeof contentItem !== 'object' || contentItem.type !== 'tool_use') {
        continue
      }

      const toolName = 'name' in contentItem ? (contentItem.name as string) : undefined
      if (!toolName) {
        continue
      }

      // Check if this is a code-changing tool
      const codeChangingTools = ['Edit', 'Write', 'MultiEdit', 'NotebookEdit', 'Bash']
      if (codeChangingTools.includes(toolName)) {
        await this.markServerDirtyAndEmitEvent(server, taskId, toolName, contentItem)
        break // Only mark dirty once per message
      }
    }
  }

  /**
   * Mark server dirty and emit code change event
   */
  private async markServerDirtyAndEmitEvent(
    server: PreviewServer,
    taskId: string,
    toolName: string,
    contentItem: unknown,
  ): Promise<void> {
    this.serviceLogger.debug(
      {
        taskId,
        toolName,
      },
      'Code-changing tool detected, marking server dirty',
    )

    // Extract file paths if available
    const filePaths = this.extractFilePaths(contentItem)

    // Mark server dirty
    server.markDirty()

    // Emit code change event
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: 'preview',
      subType: 'context-dirty',
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: {
        toolName,
        filePaths: filePaths.length > 0 ? filePaths : undefined,
      },
    })
  }

  /**
   * Extract file paths from tool input
   */
  private extractFilePaths(contentItem: unknown): string[] {
    const filePaths: string[] = []

    if (
      !contentItem ||
      typeof contentItem !== 'object' ||
      !('input' in contentItem) ||
      !contentItem.input
    ) {
      return filePaths
    }

    const input = contentItem.input
    if (typeof input !== 'object') {
      return filePaths
    }

    if ('file_path' in input && typeof input.file_path === 'string') {
      filePaths.push(input.file_path)
    }
    if ('notebook_path' in input && typeof input.notebook_path === 'string') {
      filePaths.push(input.notebook_path)
    }

    return filePaths
  }

  /**
   * Start a preview server for a task
   */
  async start(config: PreviewServerConfig): Promise<void> {
    const taskId = config.serverId
    this.serviceLogger.debug(
      {
        taskId,
        workspacePath: config.workspacePath,
      },
      'Starting preview server',
    )

    try {
      // Track server starting
      previewServerTotal.labels({ status: 'starting' }).inc()

      // Create new PreviewServer instance with this service as lifecycle sink
      const server = new PreviewServer(config, this)

      // Register server IMMEDIATELY so streamLogs() can find it
      this.servers.set(taskId, server)

      // Start the server (emits events via lifecycle sink)
      await server.start()

      // Track successful start
      previewServerTotal.labels({ status: 'started' }).inc()
      activePreviewServers.inc()

      this.serviceLogger.debug({ taskId }, 'Preview server start initiated')
    } catch (error) {
      // Track failed start
      previewServerTotal.labels({ status: 'failed' }).inc()

      // Remove from map on failure
      this.servers.delete(taskId)

      this.serviceLogger.error({ err: error as Error, taskId }, 'Failed to start preview server')
      throw error
    }
  }

  /**
   * Stop a preview server
   */
  async stop(taskId: string): Promise<void> {
    this.serviceLogger.debug({ taskId }, 'Stopping preview server')

    const server = this.servers.get(taskId)
    if (!server) {
      this.serviceLogger.debug({ taskId }, 'No server to stop')
      return
    }

    try {
      await server.stop()
      this.servers.delete(taskId)

      // Track successful stop
      previewServerTotal.labels({ status: 'stopped' }).inc()
      activePreviewServers.dec()

      this.serviceLogger.info({ taskId }, 'Preview server stopped')
    } catch (error) {
      this.serviceLogger.error({ err: error as Error, taskId }, 'Error stopping preview server')
      // Force cleanup
      this.servers.delete(taskId)
      activePreviewServers.dec()
      throw error
    }
  }

  /**
   * Restart a preview server
   */
  async restart(taskId: string, config: PreviewServerConfig): Promise<void> {
    this.serviceLogger.info({ taskId }, 'Restarting preview server')

    // Track restart
    previewServerTotal.labels({ status: 'restarted' }).inc()

    // Stop if running
    await this.stop(taskId)

    // Start fresh
    await this.start(config)
  }

  /**
   * Get preview server status
   * Returns runtime status if server is active, otherwise loads from workspace snapshot
   * Validates PID to detect stale state and updates snapshot if process is dead
   */
  async getStatus(taskId: string): Promise<ServerStatus> {
    const server = this.servers.get(taskId)
    if (server) {
      const status = server.getStatus()
      return toServerStatus(status)
    }

    // No active server - load status from workspace snapshot
    try {
      const snapshot = await this.runtimeManager.getSnapshot(taskId)

      if (snapshot?.preview) {
        const { status, pid, proxyUrl, lastStartedAt } = snapshot.preview

        // Invalid state: status='started' but no PID - this should never happen
        if (status === 'started' && !pid) {
          this.serviceLogger.warn(
            {
              taskId,
              status,
            },
            'Invalid state: status is started but no PID',
          )

          // Fix the invalid state - a server can't be 'started' without a PID
          await this.runtimeManager.updatePreviewState(taskId, {
            status: 'stopped',
            pid: undefined,
            endpoint: undefined,
            proxyUrl: undefined,
            lastStoppedAt: new Date(),
          })

          return {
            status: 'stopped',
            pid: undefined,
            url: undefined,
            uptimeSeconds: 0,
          }
        }

        // Validate PID if status shows server as started (and we have a PID)
        if (status === 'started' && pid) {
          const processAlive = isProcessAlive(pid)

          if (!processAlive) {
            // Process is dead but snapshot shows 'started' - fix stale state
            this.serviceLogger.warn(
              {
                taskId,
                pid,
                status,
              },
              'Detected stale preview server state - process is dead',
            )

            // Update snapshot to reflect reality
            await this.runtimeManager.updatePreviewState(taskId, {
              status: 'stopped',
              pid: undefined,
              endpoint: undefined,
              proxyUrl: undefined,
              lastStoppedAt: new Date(),
            })

            // Return stopped status
            return {
              status: 'stopped',
              pid: undefined,
              url: undefined,
              uptimeSeconds: 0,
            }
          }
        }

        // Status is accurate - return from snapshot
        return {
          status,
          pid,
          url: proxyUrl,
          uptimeSeconds: lastStartedAt
            ? Math.floor((Date.now() - lastStartedAt.getTime()) / 1000)
            : 0,
          dirtyContext: snapshot.preview.dirtyContext,
        }
      }
    } catch (error) {
      this.serviceLogger.debug({ taskId, error }, 'Failed to load workspace snapshot')
    }

    // Default to stopped
    return {
      status: 'stopped',
      pid: undefined,
      url: undefined,
      uptimeSeconds: 0,
    }
  }

  /**
   * Get internal server URL for backend services (Playwright, etc.)
   * This URL bypasses authentication - use only for internal tool/agent access
   * Returns undefined if server is not running
   */
  getInternalUrl(taskId: string): string | undefined {
    const server = this.servers.get(taskId)
    if (server) {
      const status = server.getStatus()
      return status.internalUrl
    }
    return undefined
  }

  /**
   * Get preview server logs
   */
  async getLogs(taskId: string, lines = 100): Promise<string> {
    const server = this.servers.get(taskId)
    if (!server) {
      return 'No preview server running'
    }

    return await server.getLogs(lines)
  }

  /**
   * Stream preview server logs via async generator
   * Only works when server is actively running
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    const server = this.servers.get(taskId)

    if (!server) {
      // No server running - yield error event and exit
      yield {
        type: 'error',
        data: {
          error: 'Preview server is not running',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    // Stream logs from the running server
    yield* server.streamLogs()
  }

  /**
   * Get preview server port
   */
  getPort(taskId: string): number | undefined {
    const server = this.servers.get(taskId)
    if (!server) {
      return undefined
    }

    // Extract port from internal URL if available
    const status = server.getStatus()
    if (status.internalUrl) {
      try {
        const url = new URL(status.internalUrl)
        return Number.parseInt(url.port, 10)
      } catch {
        return undefined
      }
    }
    return undefined
  }

  /**
   * Mark a server's workspace as dirty (code changes detected)
   */
  async markServerDirty(taskId: string): Promise<void> {
    const server = this.servers.get(taskId)
    if (server) {
      server.markDirty()
    } else {
      // If server not running, update workspace state directly
      await this.runtimeManager.updatePreviewState(taskId, {
        dirtyContext: true,
      })
    }
  }

  /**
   * Clear a server's dirty context flag
   */
  async clearServerDirty(taskId: string): Promise<void> {
    const server = this.servers.get(taskId)
    if (server) {
      server.clearDirty()
    } else {
      // If server not running, update workspace state directly
      await this.runtimeManager.updatePreviewState(taskId, {
        dirtyContext: false,
      })
    }
  }

  /**
   * Check if a server's workspace context is dirty
   */
  async isServerDirty(taskId: string): Promise<boolean> {
    const server = this.servers.get(taskId)
    if (server) {
      return server.isDirty()
    }

    // If server not running, check workspace state
    try {
      const snapshot = await this.runtimeManager.getSnapshot(taskId)
      return snapshot?.preview?.dirtyContext ?? false
    } catch {
      return false
    }
  }

  /**
   * Cleanup - stop all active servers
   * Called on application shutdown
   * Uses error handling to ensure all servers are stopped even if some fail
   */
  async cleanup(): Promise<void> {
    this.serviceLogger.info(
      {
        activeServers: this.servers.size,
      },
      'Cleaning up all preview servers',
    )

    // Unsubscribe from events
    if (this.eventUnsubscribe) {
      this.eventUnsubscribe()
      this.serviceLogger.debug('Unsubscribed from agent events')
    }

    const stopPromises = Array.from(this.servers.keys()).map(async (taskId) => {
      try {
        await this.stop(taskId)
      } catch (error) {
        // Log error but don't throw - continue cleanup for remaining servers
        this.serviceLogger.error(
          {
            err: error as Error,
            taskId,
          },
          'Error stopping preview server during cleanup',
        )
      }
    })

    await Promise.allSettled(stopPromises)

    this.serviceLogger.info('Preview server cleanup complete')
  }
}
