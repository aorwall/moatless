import { randomUUID } from 'node:crypto'
import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { isProcessAlive } from '../../../utils/process.utils.js'
import { EventService } from '../../events/services/event.service.js'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service.js'
import type { PreviewState } from '../../workspace/types/workspace-runtime.types.js'
import { PreviewServer } from '../runtime/preview-server.js'
import type {
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerStatus,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
} from '../types/index.js'

/**
 * Convert PreviewServerStatus → ServerStatus
 * Maps preview server status to moatless-specific status
 */
function toServerStatus(status: PreviewServerStatus): ServerStatus {
  return {
    status: status.status,
    pid: status.pid,
    url: status.proxyUrl || status.url, // Use proxy URL if available, otherwise direct server URL
    uptimeSeconds: status.uptimeSeconds || 0,
    currentPath: status.currentPath,
    error: status.error,
    logs: status.logs,
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

  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(EventService) private readonly eventService: EventService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-runtime' })
  }

  /**
   * PreviewServerLifecycleSink implementation - register server snapshot
   */
  async register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void> {
    this.serviceLogger.debug('Registering preview server state', {
      serverId,
      status: snapshot.status,
    })
    await this.runtimeManager.updatePreviewState(serverId, {
      status: snapshot.status,
      port: snapshot.port,
      endpoint: snapshot.url,
      proxyUrl: snapshot.proxyUrl,
      pid: snapshot.pid,
      inspectorEnabled: snapshot.inspectorEnabled,
      lastError: snapshot.error,
    })
  }

  /**
   * PreviewServerLifecycleSink implementation - update server state
   */
  async update(serverId: string, changes: PreviewServerUpdate): Promise<void> {
    this.serviceLogger.debug('Updating preview server state', { serverId, changes })

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
    if (changes.url !== undefined) {
      updates.endpoint = changes.url
    }
    if (changes.proxyUrl !== undefined) {
      updates.proxyUrl = changes.proxyUrl
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
    this.serviceLogger.debug('Removing preview server state', { serverId })
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
    this.serviceLogger.debug('Preview server status changed, emitting event', {
      serverId,
      status: statusInfo.status,
    })

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
          url: statusInfo.url,
          error: statusInfo.error,
          uptimeSeconds: statusInfo.uptimeSeconds,
        },
      })

      this.serviceLogger.debug('ServerEvent emitted successfully', {
        serverId,
        status: statusInfo.status,
      })
    } catch (error) {
      // Log error but don't throw - event emission should not break server lifecycle
      this.serviceLogger.error('Failed to emit ServerEvent', error as Error, {
        serverId,
        status: statusInfo.status,
      })
    }
  }

  /**
   * Start a preview server for a task
   */
  async start(config: PreviewServerConfig): Promise<void> {
    const taskId = config.serverId
    this.serviceLogger.info('Starting preview server', {
      taskId,
      workspacePath: config.workspacePath,
    })

    try {
      // Create new PreviewServer instance with this service as lifecycle sink
      const server = new PreviewServer(config, this)

      // Register server IMMEDIATELY so streamLogs() can find it
      this.servers.set(taskId, server)

      // Start the server (emits events via lifecycle sink)
      await server.start()

      this.serviceLogger.info('Preview server start initiated', { taskId })
    } catch (error) {
      // Remove from map on failure
      this.servers.delete(taskId)

      this.serviceLogger.error('Failed to start preview server', error as Error, { taskId })
      throw error
    }
  }

  /**
   * Stop a preview server
   */
  async stop(taskId: string): Promise<void> {
    this.serviceLogger.info('Stopping preview server', { taskId })

    const server = this.servers.get(taskId)
    if (!server) {
      this.serviceLogger.debug('No server to stop', { taskId })
      return
    }

    try {
      await server.stop()
      this.servers.delete(taskId)
      this.serviceLogger.info('Preview server stopped', { taskId })
    } catch (error) {
      this.serviceLogger.error('Error stopping preview server', error as Error, { taskId })
      // Force cleanup
      this.servers.delete(taskId)
      throw error
    }
  }

  /**
   * Restart a preview server
   */
  async restart(taskId: string, config: PreviewServerConfig): Promise<void> {
    this.serviceLogger.info('Restarting preview server', { taskId })

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
        const { status, pid, endpoint, lastStartedAt } = snapshot.preview

        // Invalid state: status='started' but no PID - this should never happen
        if (status === 'started' && !pid) {
          this.serviceLogger.warn('Invalid state: status is started but no PID', {
            taskId,
            status,
          })

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
            this.serviceLogger.warn('Detected stale preview server state - process is dead', {
              taskId,
              pid,
              status,
            })

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
          url: endpoint,
          uptimeSeconds: lastStartedAt
            ? Math.floor((Date.now() - lastStartedAt.getTime()) / 1000)
            : 0,
        }
      }
    } catch (error) {
      this.serviceLogger.debug('Failed to load workspace snapshot', { taskId, error })
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
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    const server = this.servers.get(taskId)
    if (!server) {
      yield {
        type: 'error',
        data: {
          error: 'No preview server running',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

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

    // Extract port from URL if available
    const status = server.getStatus()
    if (status.url) {
      try {
        const url = new URL(status.url)
        return Number.parseInt(url.port, 10)
      } catch {
        return undefined
      }
    }
    return undefined
  }

  /**
   * Cleanup - stop all active servers
   * Called on application shutdown
   * Uses error handling to ensure all servers are stopped even if some fail
   */
  async cleanup(): Promise<void> {
    this.serviceLogger.info('Cleaning up all preview servers', {
      activeServers: this.servers.size,
    })

    const stopPromises = Array.from(this.servers.keys()).map(async (taskId) => {
      try {
        await this.stop(taskId)
      } catch (error) {
        // Log error but don't throw - continue cleanup for remaining servers
        this.serviceLogger.error('Error stopping preview server during cleanup', error as Error, {
          taskId,
        })
      }
    })

    await Promise.allSettled(stopPromises)

    this.serviceLogger.info('Preview server cleanup complete')
  }
}
