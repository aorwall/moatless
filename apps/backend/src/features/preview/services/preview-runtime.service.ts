import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { toServerStatus } from '../../../adapters/preview-adapter.js'
import { logger } from '../../../utils/logger.js'
import { PreviewServer } from '../runtime/preview-server.js'
import type { PreviewServerConfig } from '../types/index.js'
import { PreviewRegistryService } from './preview-registry.service.js'

/**
 * PreviewRuntimeService - Manages active preview server instances
 *
 * Responsibilities:
 * - Maintain map of taskId → PreviewServer instances
 * - Start/stop/restart preview servers
 * - Stream logs from servers
 * - Cleanup on shutdown
 *
 * Design:
 * - Stateless regarding workspace lifecycle (state in snapshots)
 * - Creates PreviewServer instances with registry as lifecycle sink
 * - Delegates process management to PreviewServer
 */
@injectable()
export class PreviewRuntimeService {
  private readonly servers = new Map<string, PreviewServer>()
  private readonly serviceLogger: typeof logger

  constructor(@inject(PreviewRegistryService) private readonly registry: PreviewRegistryService) {
    this.serviceLogger = logger.child({ service: 'preview-runtime' })
  }

  /**
   * Start a preview server for a task
   */
  async start(config: PreviewServerConfig): Promise<ServerStatus> {
    const taskId = config.serverId
    this.serviceLogger.info('Starting preview server', {
      taskId,
      workspacePath: config.workspacePath,
    })

    try {
      // Create new PreviewServer instance with registry as lifecycle sink
      const server = new PreviewServer(config, this.registry)

      // Start the server
      const status = await server.start()

      // Store in active servers map
      this.servers.set(taskId, server)

      this.serviceLogger.info('Preview server started', {
        taskId,
        status: status.status,
        url: status.url,
      })

      return toServerStatus(status)
    } catch (error) {
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
  async restart(taskId: string, config: PreviewServerConfig): Promise<ServerStatus> {
    this.serviceLogger.info('Restarting preview server', { taskId })

    // Stop if running
    await this.stop(taskId)

    // Start fresh
    return await this.start(config)
  }

  /**
   * Get preview server status
   */
  getStatus(taskId: string): ServerStatus {
    const server = this.servers.get(taskId)
    if (!server) {
      return {
        status: 'stopped',
        pid: undefined,
        url: undefined,
        uptimeSeconds: 0,
      }
    }

    const status = server.getStatus()
    return toServerStatus(status)
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
  *streamLogs(taskId: string): Generator<{
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
   */
  async cleanup(): Promise<void> {
    this.serviceLogger.info('Cleaning up all preview servers', {
      activeServers: this.servers.size,
    })

    const stopPromises = Array.from(this.servers.keys()).map((taskId) => this.stop(taskId))

    await Promise.allSettled(stopPromises)

    this.serviceLogger.info('Preview server cleanup complete')
  }
}
