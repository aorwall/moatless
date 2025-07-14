import { execSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import * as net from 'node:net'
import path from 'node:path'
import { TaskNotFoundError, WorkspaceError } from '@moatless/types'
import type {
  PreviewEnvironmentMetadata,
  PreviewEnvironmentOptions,
  ProcessValidation,
} from '@moatless/types/types/preview-environment'
import type { ServerStatus } from '@moatless/types/types/workspace'
import { logger } from '../utils/logger.js'
import { ServerHandler } from '../workspaces/server-handler.js'
import type { PreviewProxyService } from './preview-proxy.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export class PreviewEnvironmentService {
  private runningServers: Map<string, ServerHandler> = new Map()
  private runningProxies: Map<string, PreviewProxyService> = new Map()
  private readonly previewEnvironmentsPath: string

  constructor(
    private storage: StorageBackend,
    private workspaceService: WorkspaceService,
  ) {
    // Initialize preview environments storage path
    this.previewEnvironmentsPath = path.join(process.cwd(), 'data', 'preview-environments')
    this.initializeStorage()
  }

  private async initializeStorage(): Promise<void> {
    try {
      await fs.mkdir(this.previewEnvironmentsPath, { recursive: true })
      // Load and validate existing preview environment metadata on startup
      await this.loadAndValidateExistingPreviewEnvironments()
    } catch (error) {
      logger.error('Failed to initialize preview environment storage', error as Error)
    }
  }

  /**
   * Load existing preview environment metadata and validate processes on service startup
   */
  private async loadAndValidateExistingPreviewEnvironments(): Promise<void> {
    try {
      const files = await fs.readdir(this.previewEnvironmentsPath)
      const jsonFiles = files.filter((f) => f.endsWith('.json'))

      for (const file of jsonFiles) {
        const taskId = file.replace('.json', '')
        try {
          const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
          if (metadata) {
            // Validate if the process is still running
            const validation = await this.validateProcess(metadata.pid, metadata.command)

            if (validation.exists && validation.isCorrectProcess) {
              logger.info('Recovered running preview environment server', {
                taskId,
                pid: metadata.pid,
                port: metadata.port,
              })

              // Create a ServerHandler instance for the recovered server
              const handler = new ServerHandler(
                metadata.workspacePath,
                metadata.command,
                metadata.envVars || {},
              )

              // Restore the handler's state
              this.restoreServerHandler(handler, metadata)
              this.runningServers.set(taskId, handler)
            } else {
              logger.info('Cleaning up orphaned preview environment metadata', {
                taskId,
                pid: metadata.pid,
                validation,
              })
              await this.deletePreviewEnvironmentMetadata(taskId)
            }
          }
        } catch (error) {
          logger.error(`Failed to validate preview environment for task ${taskId}`, error as Error)
        }
      }
    } catch (error) {
      logger.error('Failed to load existing preview environments', error as Error)
    }
  }

  /**
   * Start a preview environment server for a task
   */
  async startServer(taskId: string): Promise<ServerStatus> {
    const previewLogger = logger.child({ taskId })

    // Validate task exists
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Check if server is already running
    const existingServer = this.runningServers.get(taskId)
    if (existingServer) {
      const status = existingServer.getStatus()
      if (status.running) {
        previewLogger.info('Server already running', { port: status.url })
        return status
      }
    }

    // Get workspace to access environment configuration
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
    const environment = workspace.environmentConfig
    const previewConfig = environment.previewConfig

    // Use preview config's server command if available, otherwise fall back to codeExecution
    const serverCommand =
      previewConfig?.serverStartCommand || environment.codeExecution.serverCommand

    if (!serverCommand) {
      throw new WorkspaceError('No server command configured for this environment', taskId)
    }

    // Create server handler
    const workspacePath = workspace.getWorkspacePath()
    const handler = new ServerHandler(
      workspacePath,
      serverCommand,
      environment.codeExecution.envVars || {},
    )

    previewLogger.info('Starting preview environment server', {
      command: serverCommand,
      framework: previewConfig?.framework,
      workspacePath,
    })

    try {
      // Start the server first
      const result = await handler.start()

      // Store the handler
      this.runningServers.set(taskId, handler)

      // Save metadata to disk (successful startup, clear any previous error state)
      const metadata: PreviewEnvironmentMetadata = {
        taskId,
        pid: result.pid,
        port: result.port,
        startTime: new Date(),
        command: serverCommand,
        workspacePath,
        logFile: result.log_file,
        processStartTime: Date.now(),
        envVars: environment.codeExecution.envVars,
        failed: false, // Explicitly mark as not failed
        error: undefined, // Clear any previous error
        logs: undefined, // Clear any previous error logs
      }

      await this.savePreviewEnvironmentMetadata(taskId, metadata)

      previewLogger.info('Preview environment server started successfully', {
        port: result.port,
        pid: result.pid,
      })

      return {
        running: true,
        configured: true,
        url: `http://localhost:${result.port}`,
        pid: result.pid,
        uptimeSeconds: 0,
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      previewLogger.error('Failed to start preview environment server', {
        error: errorMessage,
        command: serverCommand,
        workspacePath,
      })

      // Try to get server logs for better error context
      let logs = ''
      try {
        logs = await handler.getLogs(50)
      } catch (logError) {
        previewLogger.debug('Could not retrieve server logs', {
          error: logError instanceof Error ? logError.message : String(logError),
        })
      }

      // Save error metadata to disk for later retrieval
      const errorMetadata: PreviewEnvironmentMetadata = {
        taskId,
        pid: -1, // No valid PID for failed startup
        port: -1, // No valid port for failed startup
        startTime: new Date(),
        command: serverCommand,
        workspacePath,
        logFile: '',
        processStartTime: Date.now(),
        envVars: environment.codeExecution.envVars,
        failed: true,
        error: errorMessage,
        logs: logs || 'No logs available',
      }

      await this.savePreviewEnvironmentMetadata(taskId, errorMetadata)

      // Return failed state instead of throwing
      return {
        running: false,
        configured: true,
        uptimeSeconds: 0,
        error: errorMessage,
        logs: logs || 'No logs available',
      }
    }
  }

  /**
   * Stop a preview environment server
   */
  async stopServer(taskId: string, options?: PreviewEnvironmentOptions): Promise<boolean> {
    const previewLogger = logger.child({ taskId })

    const handler = this.runningServers.get(taskId)
    if (!handler) {
      previewLogger.debug('No server handler found')
      // Check if we have metadata for a potentially orphaned process
      const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
      if (metadata) {
        previewLogger.info('Found orphaned server metadata, attempting cleanup', {
          pid: metadata.pid,
        })
        await this.killOrphanedProcess(metadata.pid, options?.forceKill)
        await this.deletePreviewEnvironmentMetadata(taskId)
        return true
      }
      return false
    }

    try {
      const stopped = await handler.stop(options?.timeout)
      if (stopped) {
        this.runningServers.delete(taskId)
        await this.deletePreviewEnvironmentMetadata(taskId)

        // Stop proxy if running
        const proxy = this.runningProxies.get(taskId)
        if (proxy) {
          await proxy.stop()
          this.runningProxies.delete(taskId)
        }

        previewLogger.info('Preview environment server stopped successfully')
      }
      return stopped
    } catch (error) {
      previewLogger.error('Error stopping preview environment server', error as Error)
      // Force cleanup
      this.runningServers.delete(taskId)
      await this.deletePreviewEnvironmentMetadata(taskId)

      // Stop proxy if running
      const proxy = this.runningProxies.get(taskId)
      if (proxy) {
        try {
          await proxy.stop()
        } catch (proxyError) {
          previewLogger.error('Failed to stop proxy during error handling', proxyError as Error)
        }
        this.runningProxies.delete(taskId)
      }

      return false
    }
  }

  /**
   * Restart a preview environment server
   */
  async restartServer(taskId: string): Promise<ServerStatus> {
    const previewLogger = logger.child({ taskId, operation: 'restartServer' })
    previewLogger.info('Restarting preview environment server')

    try {
      // First stop the server if it's running
      const stopped = await this.stopServer(taskId)
      previewLogger.debug(`Server stopped: ${stopped}`)

      // Wait a moment for the server to fully stop
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Then start it again
      const result = await this.startServer(taskId)
      previewLogger.info('Preview environment server restarted successfully')

      return result
    } catch (error) {
      previewLogger.error('Error restarting preview environment server', error as Error)
      throw error
    }
  }

  /**
   * Get server status for a task
   */
  async getServerStatus(taskId: string): Promise<ServerStatus> {
    // Validate task exists
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Check if we can manage server for this task
    const canManage = await this.workspaceService.canManageServer(taskId)
    if (!canManage) {
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }

    // Get workspace to check configuration
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
    const configured = Boolean(workspace.environmentConfig.codeExecution.serverCommand)

    // Check running servers
    const handler = this.runningServers.get(taskId)
    if (handler) {
      const status = handler.getStatus(configured)

      // Perform ping check if server is running
      if (status.running && status.url) {
        const port = this.extractPortFromUrl(status.url)
        const responsive = port ? await this.pingServer(port) : false

        return {
          ...status,
          responsive,
        } as ServerStatus
      }

      return status
    }

    // Check for orphaned server via metadata
    const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
    if (metadata) {
      // Check if this is error metadata from a failed startup
      if (metadata.failed) {
        return {
          running: false,
          configured,
          uptimeSeconds: 0,
          error: metadata.error,
          logs: metadata.logs,
        } as ServerStatus
      }

      const validation = await this.validateProcess(metadata.pid, metadata.command)
      if (validation.exists && validation.isCorrectProcess) {
        // Process is still running, recover it
        const handler = new ServerHandler(
          metadata.workspacePath,
          metadata.command,
          metadata.envVars || {},
        )

        this.restoreServerHandler(handler, metadata)
        this.runningServers.set(taskId, handler)

        const responsive = await this.pingServer(metadata.port)
        const uptime = Math.floor((Date.now() - metadata.startTime.getTime()) / 1000)

        return {
          running: true,
          configured,
          url: `http://localhost:${metadata.port}`,
          pid: metadata.pid,
          uptimeSeconds: uptime,
          responsive,
        } as ServerStatus
      } else {
        // Clean up invalid metadata
        await this.deletePreviewEnvironmentMetadata(taskId)
      }
    }

    return {
      running: false,
      configured,
      uptimeSeconds: 0,
    }
  }

  /**
   * Get server logs
   */
  async getServerLogs(taskId: string, lines = 100): Promise<string> {
    const handler = this.runningServers.get(taskId)
    if (!handler) {
      // Try to get logs from metadata
      const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
      if (metadata?.logFile) {
        try {
          const content = await fs.readFile(metadata.logFile, 'utf-8')
          const allLines = content.split('\n')
          return allLines.slice(-lines).join('\n')
        } catch (_error) {
          return 'Failed to read log file'
        }
      }
      return 'No server logs available'
    }

    return await handler.getLogs(lines)
  }

  /**
   * Validate if a process is still running and is the correct one
   */
  private async validateProcess(pid: number, expectedCommand: string): Promise<ProcessValidation> {
    try {
      if (process.platform === 'win32') {
        // Windows: Use wmic to get process info
        try {
          const result = execSync(
            `wmic process where ProcessId=${pid} get CommandLine,CreationDate /format:list`,
            { encoding: 'utf-8' },
          )

          const lines = result.split('\n').filter((line) => line.trim())
          const commandLine = lines.find((line) => line.startsWith('CommandLine='))

          if (commandLine) {
            const command = commandLine.split('=')[1]
            return {
              exists: true,
              isCorrectProcess: command.includes(expectedCommand.split(' ')[0]),
              command,
            }
          }
        } catch {
          return { exists: false, isCorrectProcess: false }
        }
      } else {
        // Unix/Linux/Mac: Use ps to get process info
        try {
          const result = execSync(`ps -p ${pid} -o command=`, { encoding: 'utf-8' })
          const command = result.trim()

          return {
            exists: true,
            isCorrectProcess: command.includes(expectedCommand.split(' ')[0]),
            command,
          }
        } catch {
          return { exists: false, isCorrectProcess: false }
        }
      }
    } catch (error) {
      logger.debug('Error validating process', { pid, error })
      return { exists: false, isCorrectProcess: false }
    }

    return { exists: false, isCorrectProcess: false }
  }

  /**
   * Extract port number from URL string
   */
  private extractPortFromUrl(url?: string): number | null {
    if (!url) return null

    try {
      // Handle both formats: "http://localhost:port" and "localhost:port"
      const parts = url.split(':')

      if (parts.length >= 2) {
        // If URL starts with http://, port is at index 2
        // If URL is just localhost:port, port is at index 1
        const portString = url.startsWith('http://') ? parts[2] : parts[parts.length - 1]
        const port = parseInt(portString, 10)

        // Validate port range
        if (Number.isNaN(port) || port < 0 || port > 65535) {
          return null
        }

        return port
      }
    } catch (error) {
      logger.debug('Error extracting port from URL', { url, error })
    }

    return null
  }

  /**
   * Ping a server to check if it's responsive
   */
  private async pingServer(port: number, timeout = 1000): Promise<boolean> {
    return new Promise((resolve) => {
      const client = new net.Socket()
      client.setTimeout(timeout)

      client.on('connect', () => {
        client.destroy()
        resolve(true)
      })

      client.on('timeout', () => {
        client.destroy()
        resolve(false)
      })

      client.on('error', () => {
        resolve(false)
      })

      client.connect(port, '127.0.0.1')
    })
  }

  /**
   * Kill an orphaned process
   */
  private async killOrphanedProcess(pid: number, force = false): Promise<void> {
    try {
      if (process.platform === 'win32') {
        execSync(`taskkill ${force ? '/F' : ''} /PID ${pid}`)
      } else {
        process.kill(pid, force ? 'SIGKILL' : 'SIGTERM')
      }
    } catch (error) {
      logger.debug('Failed to kill orphaned process', { pid, error })
    }
  }

  /**
   * Restore ServerHandler state from metadata
   */
  private restoreServerHandler(handler: ServerHandler, metadata: PreviewEnvironmentMetadata): void {
    // Use private property access to restore state
    // This is a bit hacky but necessary for recovery
    ;(handler as unknown as { port: number }).port = metadata.port
    ;(handler as unknown as { pid: number }).pid = metadata.pid
    ;(handler as unknown as { logFile: string }).logFile = metadata.logFile
    ;(handler as unknown as { startTime: number }).startTime = metadata.startTime.getTime()

    // Create a mock process object with event emitter functionality
    const mockProcess = {
      exitCode: null,
      on: (_event: string, _callback: (...args: unknown[]) => void) => {},
      kill: (_signal?: string) => {},
    }
    ;(handler as unknown as { process: typeof mockProcess }).process = mockProcess
  }

  /**
   * Save preview environment metadata to disk
   */
  private async savePreviewEnvironmentMetadata(
    taskId: string,
    metadata: PreviewEnvironmentMetadata,
  ): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    await fs.writeFile(filePath, JSON.stringify(metadata, null, 2), 'utf-8')
  }

  /**
   * Load preview environment metadata from disk
   */
  private async loadPreviewEnvironmentMetadata(
    taskId: string,
  ): Promise<PreviewEnvironmentMetadata | null> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    try {
      const content = await fs.readFile(filePath, 'utf-8')
      const data = JSON.parse(content)
      // Convert date strings back to Date objects
      data.startTime = new Date(data.startTime)
      return data
    } catch {
      return null
    }
  }

  /**
   * Delete preview environment metadata from disk
   */
  private async deletePreviewEnvironmentMetadata(taskId: string): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    try {
      await fs.unlink(filePath)
    } catch {
      // Ignore errors if file doesn't exist
    }
  }

  /**
   * Stop all running servers (useful for shutdown)
   */
  async stopAllServers(): Promise<void> {
    const taskIds = Array.from(this.runningServers.keys())
    const proxyTaskIds = Array.from(this.runningProxies.keys())
    const allTaskIds = [...new Set([...taskIds, ...proxyTaskIds])]

    logger.info('Stopping all preview environment servers', {
      servers: taskIds.length,
      proxies: proxyTaskIds.length,
      total: allTaskIds.length,
    })

    // Stop servers (which includes cleanup and proxy stopping)
    const stopPromises = allTaskIds.map((taskId) => this.stopServer(taskId))
    await Promise.allSettled(stopPromises)

    this.runningServers.clear()
    this.runningProxies.clear()
  }

  /**
   * Graceful shutdown - stops all servers and cleans up metadata files
   */
  async gracefulShutdown(): Promise<void> {
    logger.info('Starting graceful shutdown of preview environment service')

    try {
      // Get all running servers AND check preview env directory for metadata files
      const runningTaskIds = Array.from(this.runningServers.keys())
      const metadataTaskIds = await this.getAllMetadataFiles()

      // Combine and deduplicate task IDs from both sources
      const allTaskIds = [...new Set([...runningTaskIds, ...metadataTaskIds])]

      logger.info('Preview environment shutdown status', {
        runningServers: runningTaskIds.length,
        metadataFiles: metadataTaskIds.length,
        totalEnvironments: allTaskIds.length,
      })

      if (allTaskIds.length > 0) {
        logger.info('Stopping all preview environments', { taskIds: allTaskIds })

        const stopPromises = allTaskIds.map(async (taskId) => {
          try {
            // Try graceful stop first with 5 second timeout
            const stopped = await this.stopServer(taskId, { timeout: 5000 })
            if (!stopped) {
              // Force stop if graceful failed
              logger.warn('Graceful stop failed, forcing shutdown', { taskId })
              await this.stopServer(taskId, { timeout: 2000, forceKill: true })
            }
            return { taskId, success: true }
          } catch (error) {
            logger.error('Failed to stop server during shutdown', { taskId, error })
            return { taskId, success: false }
          }
        })

        const results = await Promise.allSettled(stopPromises)
        const successful = results.filter((r) => r.status === 'fulfilled' && r.value.success).length

        logger.info('Preview environment shutdown complete', {
          attempted: allTaskIds.length,
          successful,
          failed: allTaskIds.length - successful,
        })
      }

      // Clear the running servers map
      this.runningServers.clear()

      logger.info('Preview environment service shutdown complete')
    } catch (error) {
      logger.error('Error during graceful shutdown', error as Error)
      throw error
    }
  }

  /**
   * Get all metadata files in the preview environments directory
   */
  private async getAllMetadataFiles(): Promise<string[]> {
    try {
      const files = await fs.readdir(this.previewEnvironmentsPath)
      return files.filter((f) => f.endsWith('.json')).map((f) => f.replace('.json', ''))
    } catch (error) {
      logger.debug('Failed to read preview environments directory', {
        error: error instanceof Error ? error.message : String(error),
      })
      return []
    }
  }
}

export function createPreviewEnvironmentService(
  storage: StorageBackend,
  workspaceService: WorkspaceService,
): PreviewEnvironmentService {
  return new PreviewEnvironmentService(storage, workspaceService)
}
