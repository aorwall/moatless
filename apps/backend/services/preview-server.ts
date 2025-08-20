import { randomUUID } from 'node:crypto'
import { tmpdir } from 'node:os'
import path from 'node:path'
import type { RepositoryConfig, ServerStatus, ServerStatusEnum } from '@moatless/types'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
import { findHttpPortForPid } from '../utils/port-detection.js'
import { getPreviewInspectorScript } from '../utils/preview-inspector-script.js'
import type { EventService } from './event.service.js'
import { createPreviewProxy, type PreviewProxyService } from './preview-proxy.service.js'
import { ShellCommand } from './shell-command.js'

interface ServerStartResult {
  status: 'starting' | 'failed' | 'stopped'
  port: number
  pid: number
  log_file: string
  uptime_seconds: number
  error?: string
}

interface LogStreamEvent {
  type: 'chunk' | 'error' | 'complete'
  data: {
    text?: string
    error?: string
    timestamp: string
  }
}

export class PreviewServer {
  private shellCommand?: ShellCommand
  private port?: number
  private logFile?: string
  private logger = createTaskLogger('preview-server')
  private previewProxy?: PreviewProxyService
  private proxyPort?: number
  private proxyUrl?: string
  private isIntentionallyStopping = false

  // Runtime configuration - set when start() is called
  private repositoryConfig?: RepositoryConfig

  constructor(
    private taskId: string,
    private workspacePath: string,
    private eventService?: EventService,
  ) {
    this.logger.info('PreviewServer initialized', {
      taskId: this.taskId,
      workspacePath,
      hasEventService: !!this.eventService,
    })
  }

  private async emitServerEvent(status: ServerStatusEnum, error?: string): Promise<void> {
    if (!this.eventService) return

    const pid = this.shellCommand?.getPid()
    const info = this.shellCommand?.getInfo()

    try {
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        agentType: 'environment' as const,
        type: 'server' as const,
        subType: status,
        taskId: this.taskId,
        timestamp: new Date(),
        isError: status === 'failed',
        payload: {
          status,
          port: this.port,
          pid,
          url: this.port ? `http://localhost:${this.port}` : undefined,
          error,
          uptimeSeconds: info?.duration ? Math.floor(info.duration / 1000) : 0,
        },
      })

      this.logger.debug('Server event emitted', { status, taskId: this.taskId })
    } catch (eventError) {
      this.logger.error('Failed to emit server event', eventError as Error, {
        status,
        taskId: this.taskId,
      })
    }
  }

  /**
   * Start the preview server
   */
  async start(
    serverCommands: string | string[],
    envVars?: Record<string, string>,
    repositoryConfig?: RepositoryConfig,
  ): Promise<ServerStartResult> {
    this.logger.info('Starting preview server', {
      serverCommands,
      envVars: envVars ? Object.keys(envVars) : [],
    })

    // Store runtime configuration
    this.repositoryConfig = repositoryConfig

    // Clean up any existing command
    if (this.shellCommand?.isRunning()) {
      await this.stop()
    }
    this.cleanup()

    // Convert commands to array
    const commandsArray = Array.isArray(serverCommands) ? serverCommands : [serverCommands]

    // Create log file
    const timestamp = Date.now()
    const logFileName = `server-${this.taskId}-${timestamp}.log`
    this.logFile = path.join(tmpdir(), logFileName)

    try {
      // Create new shell command instance
      this.shellCommand = new ShellCommand()

      // Prepare the command
      const command = commandsArray.join(' && ')

      this.logger.info('Starting server with command', {
        command,
        workspacePath: this.workspacePath,
        logFile: this.logFile,
      })

      // Start the command in background
      await this.shellCommand.execute(command, {
        workingDirectory: this.workspacePath,
        env: envVars,
        background: true,
        logFile: this.logFile,
        timeout: 300000, // 5 minute timeout
      })

      const pid = this.shellCommand.getPid()
      if (!pid) {
        const error = 'Failed to get process PID'
        await this.emitServerEvent('failed', error)
        return {
          status: 'failed',
          port: 0,
          pid: 0,
          log_file: this.logFile,
          uptime_seconds: 0,
          error,
        }
      }

      this.logger.info('Server process started', { pid, command })

      // Start port detection in background
      this.detectPort().catch((error) => {
        this.logger.error('Port detection failed', error as Error)
      })

      // Check immediate status
      const immediateStatus = this.shellCommand.getStatus()
      if (immediateStatus === 'failed') {
        const error = 'Process failed to start'
        await this.emitServerEvent('failed', error)
        return {
          status: 'failed',
          port: 0,
          pid,
          log_file: this.logFile,
          uptime_seconds: 0,
          error,
        }
      }

      // Return immediately with PID available
      await this.emitServerEvent('starting')
      return {
        status: 'starting',
        port: 0,
        pid,
        log_file: this.logFile,
        uptime_seconds: 0,
      }
    } catch (error) {
      this.logger.error('Failed to start server', error as Error)
      const errorMsg = error instanceof Error ? error.message : String(error)
      await this.emitServerEvent('failed', errorMsg)
      throw error
    }
  }

  /**
   * Detect the port the server is running on
   */
  private async detectPort(): Promise<void> {
    const MAX_ATTEMPTS = 30 // 30 seconds max
    const INTERVAL = 1000 // Check every second

    this.logger.info('Starting port detection')

    for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt++) {
      // Check if process is still running
      if (!this.shellCommand || this.shellCommand.getStatus() !== 'running') {
        this.logger.info('Process no longer running, stopping port detection')
        break
      }

      // Get PID and try to detect port
      const pid = this.shellCommand.getPid()
      if (pid) {
        const detectedPort = await findHttpPortForPid(pid)
        if (detectedPort) {
          this.port = detectedPort
          this.logger.info('Port detected', { port: detectedPort, pid, attempt })

          // Start proxy if configured
          await this.startProxy()

          // Emit success event
          await this.emitServerEvent('started')
          return
        }
      }

      // Wait before next attempt
      await new Promise((resolve) => setTimeout(resolve, INTERVAL))
    }

    // Port detection failed or process died
    if (this.shellCommand) {
      const info = this.shellCommand.getInfo()
      const status = this.shellCommand.getStatus()

      if (!this.isIntentionallyStopping) {
        if (status === 'failed' || (info?.exitCode && info.exitCode !== 0)) {
          const error = info?.error || `Process exited with code ${info?.exitCode || 'unknown'}`
          this.logger.error('Server failed - no port detected', { error })
          await this.emitServerEvent('failed', error)
        } else if (status === 'completed') {
          this.logger.info('Server command completed without starting a server')
          await this.emitServerEvent('stopped')
        }
      }
    }
  }

  /**
   * Stop the preview server
   */
  async stop(): Promise<boolean> {
    if (!this.shellCommand) {
      this.logger.debug('No server to stop')
      return false
    }

    const status = this.shellCommand.getStatus()
    if (status !== 'running') {
      this.logger.debug('Server not running', { status })
      return false
    }

    this.logger.info('Stopping preview server', {
      pid: this.shellCommand.getPid(),
      port: this.port,
    })

    this.isIntentionallyStopping = true

    try {
      // Stop proxy first
      if (this.previewProxy) {
        this.logger.debug('Stopping preview proxy')
        await this.previewProxy.stop()
      }

      // Kill the server process
      const killed = await this.shellCommand.kill()

      if (killed) {
        await this.emitServerEvent('stopped')
        this.logger.info('Server stopped successfully')
      }

      return killed
    } finally {
      this.cleanup()
      this.isIntentionallyStopping = false
    }
  }

  /**
   * Get the current status of the server
   */
  getStatus(): ServerStatus {
    if (!this.shellCommand) {
      return { status: 'stopped' }
    }

    const commandStatus = this.shellCommand.getStatus()
    const info = this.shellCommand.getInfo()

    // If not running, determine if it failed or stopped
    if (commandStatus !== 'running') {
      const isFailure = commandStatus === 'failed' || (info?.exitCode && info.exitCode !== 0)
      const error =
        info?.error ||
        (isFailure && info?.exitCode !== undefined
          ? `Process exited with code ${info.exitCode}`
          : undefined)

      return {
        status: isFailure ? 'failed' : 'stopped',
        error,
      }
    }

    // Process is running - check if we have a port
    const uptime = info?.duration ? Math.floor(info.duration / 1000) : 0
    const pid = this.shellCommand.getPid()

    if (this.port) {
      // Server is fully running with detected port
      return {
        status: 'started',
        url: `http://localhost:${this.port}`,
        proxyUrl: this.proxyUrl,
        pid,
        uptimeSeconds: uptime,
      }
    }

    // Process is running but no port detected yet
    return {
      status: 'starting',
      pid,
      uptimeSeconds: uptime,
    }
  }

  /**
   * Get server logs
   */
  async getLogs(lines = 500): Promise<string> {
    if (!this.shellCommand) {
      return 'No server process available'
    }

    try {
      return this.shellCommand.getFormattedLogs(true, lines)
    } catch (error) {
      this.logger.error('Error reading logs:', error as Error)
      return `Error reading logs: ${error}`
    }
  }

  /**
   * Stream server logs
   */
  async *streamLogs(): AsyncGenerator<LogStreamEvent> {
    if (!this.shellCommand) {
      yield {
        type: 'error',
        data: {
          error: 'No server process available',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    try {
      // Stream logs from ShellCommand
      for await (const logEvent of this.shellCommand.streamLogs()) {
        yield {
          type: 'chunk',
          data: {
            text: `[${logEvent.type}] ${logEvent.text}`,
            timestamp: logEvent.timestamp,
          },
        }
      }

      // Send completion event
      yield {
        type: 'complete',
        data: {
          timestamp: new Date().toISOString(),
        },
      }
    } catch (error) {
      yield {
        type: 'error',
        data: {
          error: error instanceof Error ? error.message : String(error),
          timestamp: new Date().toISOString(),
        },
      }
    }
  }

  /**
   * Get the detected port
   */
  getPort(): number | undefined {
    return this.port
  }

  /**
   * Get the actual server port (same as getPort)
   */
  getActualPort(): number | undefined {
    return this.port
  }

  /**
   * Start the preview proxy server with inspector script injection
   */
  private async startProxy(): Promise<void> {
    if (!this.repositoryConfig?.previewConfig) {
      this.logger.debug('No repository config for proxy setup')
      return
    }

    const enableInspector = this.repositoryConfig.previewConfig.enableInspector !== false
    if (!enableInspector) {
      this.logger.info('Inspector disabled, skipping proxy server creation')
      return
    }

    if (!this.port) {
      this.logger.warn('Cannot start proxy without main server port')
      return
    }

    try {
      this.proxyPort = await findAvailablePort('general')
      const targetUrl = `http://localhost:${this.port}`
      const scriptContent = getPreviewInspectorScript(this.taskId)

      this.previewProxy = createPreviewProxy(targetUrl, scriptContent, this.logger, this.proxyPort)

      await this.previewProxy.start()
      this.proxyUrl = `http://localhost:${this.proxyPort}`

      this.logger.info('Preview proxy started', {
        proxyPort: this.proxyPort,
        targetPort: this.port,
        targetUrl,
      })
    } catch (error) {
      this.logger.error('Failed to start preview proxy', error as Error)
      // Don't fail the server start if proxy fails
    }
  }

  /**
   * Proxy a request to the preview server
   */
  async proxyRequest(request: {
    method: string
    path: string
    headers: Headers
    body?: string
  }): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }> {
    if (!this.port) {
      throw new Error('Preview server not running')
    }

    const url = `http://localhost:${this.port}${request.path}`

    try {
      const response = await fetch(url, {
        method: request.method,
        headers: request.headers,
        body: request.body,
      })

      const responseBody = await response.text()

      return {
        status: response.status,
        headers: response.headers,
        body: responseBody,
      }
    } catch (error) {
      this.logger.error('Failed to proxy request', error as Error, {
        url,
        method: request.method,
      })
      throw error
    }
  }

  /**
   * Clean up resources
   */
  private cleanup(): void {
    this.shellCommand = undefined
    this.port = undefined
    this.proxyPort = undefined
    this.proxyUrl = undefined
    this.previewProxy = undefined
  }
}
