import { randomUUID } from 'node:crypto'
import { tmpdir } from 'node:os'
import path from 'node:path'
import type { RepositoryConfig, ServerStatus, ServerStatusEnum } from '@moatless/types'
import { createTaskLogger } from '../utils/logger.js'
import { getPort } from '../utils/port-utils.js'
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
  private shellCommand: ShellCommand | undefined
  private port: number | undefined
  private logFile: string | undefined
  private logger
  private previewProxy: PreviewProxyService | undefined
  private proxyPort: number | undefined
  private proxyUrl: string | undefined

  // Runtime configuration - set when start() is called
  private repositoryConfig: RepositoryConfig | undefined

  constructor(
    private taskId: string,
    private workspacePath: string,
    private eventService?: EventService,
    logFile?: string,
  ) {
    // Initialize logger with actual taskId
    this.logger = createTaskLogger(taskId)

    // Use provided log file or undefined
    this.logFile = logFile

    this.logger.info('PreviewServer initialized', {
      taskId: this.taskId,
      workspacePath,
      hasEventService: !!this.eventService,
      hasLogFile: !!this.logFile,
    })
  }

  /**
   * Wait for the server to be HTTP-responsive on the allocated port
   * @param timeoutSeconds - Total time to wait in seconds (default: 30, from config or fallback)
   */
  private async waitForHttpReady(timeoutSeconds = 30): Promise<boolean> {
    if (!this.port) {
      return false
    }

    // Fixed delay between attempts
    const delayMs = 1000
    // Calculate attempts based on timeout
    const maxAttempts = Math.floor(timeoutSeconds / (delayMs / 1000))

    this.logger.info(
      { port: this.port, maxAttempts, timeoutSeconds },
      'Waiting for server to become HTTP-responsive',
    )

    for (let i = 0; i < maxAttempts; i++) {
      try {
        const response = await fetch(`http://localhost:${this.port}`, {
          signal: AbortSignal.timeout(3000),
        })
        if (response.ok) {
          this.logger.info({ port: this.port, attempts: i + 1 }, 'Server is HTTP-responsive')
          return true
        }
      } catch (_error) {
        // Log progress every 5 attempts (every ~5 seconds)
        if (i > 0 && i % 5 === 0) {
          const elapsedSeconds = Math.floor((i * delayMs) / 1000)
          this.logger.info(
            { port: this.port, attempt: i + 1, maxAttempts, elapsedSeconds },
            'Still waiting for server to start...',
          )
        } else {
          this.logger.debug(
            { port: this.port, attempt: i + 1, maxAttempts },
            'Server not ready yet',
          )
        }
        await new Promise((resolve) => setTimeout(resolve, delayMs))
      }
    }
    return false
  }

  private async emitServerEvent(status: ServerStatusEnum, error?: string): Promise<void> {
    if (!this.eventService) return

    const serverPid = this.shellCommand?.getPid()
    const info = this.shellCommand?.getInfo()

    try {
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'server' as const,
        subType: status,
        taskId: this.taskId,
        timestamp: new Date(),
        isError: status === 'failed',
        payload: {
          status,
          port: this.port,
          pid: serverPid,
          url: this.port ? `http://localhost:${this.port}` : undefined,
          error,
          uptimeSeconds: info?.duration ? Math.floor(info.duration / 1000) : 0,
        },
      })

      this.logger.debug('Server event emitted', {
        status,
        taskId: this.taskId,
      })
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
      this.cleanup() // Only do full cleanup if we had to stop a running server
    } else if (this.shellCommand) {
      // If we have a shell command but it's not running (stopped server),
      // just clean up the command reference but keep the port
      this.shellCommand = undefined
    }

    // Convert commands to array
    const commandsArray = Array.isArray(serverCommands) ? serverCommands : [serverCommands]

    // Log file should already be set from constructor
    // If not set, create a fallback temporary log file
    if (!this.logFile) {
      const timestamp = Date.now()
      const logFileName = `server-${this.taskId}-${timestamp}.log`
      this.logFile = path.join(tmpdir(), logFileName)
      this.logger.warn('No log file provided to PreviewServer, using temporary file', {
        logFile: this.logFile,
      })
    }

    // Only allocate a new port if we don't have one already
    if (!this.port) {
      this.port = await getPort()

      if (!this.port) {
        throw new Error('Failed to find available port')
      }

      this.logger.info('Allocated new port for server', { port: this.port })
    } else {
      this.logger.info('Reusing existing port for server', { port: this.port })
    }

    try {
      this.shellCommand = new ShellCommand()

      // Prepare the command - replace {PORT} placeholder if present
      const command = commandsArray
        .map((cmd) => cmd.replace(/{PORT}/g, String(this.port)))
        .join(' && ')

      this.logger.info('Starting server with command', {
        command,
        workspacePath: this.workspacePath,
        logFile: this.logFile,
        port: this.port,
      })

      // Emit 'starting' event before starting the process
      await this.emitServerEvent('starting')

      // Start the command in background with PORT environment variable
      await this.shellCommand.execute(command, {
        workingDirectory: this.workspacePath,
        env: {
          ...envVars,
          PORT: String(this.port),
        },
        background: true,
        logFile: this.logFile,
      })

      const pid = this.shellCommand.getPid()
      if (!pid) {
        const error = 'Failed to get process PID'
        this.logger.error({ command, error }, 'Failed to start server - no PID')
        await this.emitServerEvent('failed', error)
        return {
          status: 'failed',
          port: this.port,
          pid: 0,
          log_file: this.logFile,
          uptime_seconds: 0,
          error,
        }
      }

      this.logger.info({ serverPid: pid, command, port: this.port }, 'Server process started')

      // Check immediate status
      const immediateStatus = this.shellCommand.getStatus()
      if (immediateStatus === 'failed') {
        const error = 'Process failed to start'
        await this.emitServerEvent('failed', error)
        return {
          status: 'failed',
          port: this.port,
          pid,
          log_file: this.logFile,
          uptime_seconds: 0,
          error,
        }
      }

      // Start proxy if configured
      await this.startProxy()

      // Wait for server to be HTTP-responsive before emitting 'started'
      const timeoutSeconds = this.repositoryConfig?.previewConfig?.startupTimeoutSeconds ?? 30
      this.logger.info(
        { port: this.port, timeoutSeconds },
        'Waiting for server to be HTTP-responsive...',
      )
      const isReady = await this.waitForHttpReady(timeoutSeconds)

      if (!isReady) {
        const error = 'Server process started but did not become HTTP-responsive'
        this.logger.error({ port: this.port }, error)
        await this.emitServerEvent('failed', error)
        return {
          status: 'failed',
          port: this.port,
          pid,
          log_file: this.logFile,
          uptime_seconds: 0,
          error,
        }
      }

      // Now emit started event
      await this.emitServerEvent('started')

      return {
        status: 'starting',
        port: this.port,
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

    this.logger.info(
      {
        serverPid: this.shellCommand.getPid(),
        port: this.port,
      },
      'Stopping preview server',
    )

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
      // Don't release the main port on stop - we want to reuse it on restart
      // Clean up everything except the main port
      this.cleanupForStop()
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
    const shellPid = this.shellCommand.getPid()

    if (this.port) {
      // Server is running with allocated port
      return {
        status: 'started',
        url: `http://localhost:${this.port}`,
        proxyUrl: this.proxyUrl,
        pid: shellPid,
        uptimeSeconds: uptime,
        currentPath: this.previewProxy?.getCurrentPath(),
      }
    }

    // Process is running but no port allocated yet (shouldn't happen)
    return {
      status: 'starting',
      pid: shellPid,
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
   * Note: When hostname routing is enabled, this is skipped - the global HostnameProxyService handles it
   */
  private async startProxy(): Promise<void> {
    if (!this.repositoryConfig?.previewConfig) {
      this.logger.warn('No repository config for proxy setup')
      return
    }

    const enableInspector = this.repositoryConfig.previewConfig.enableInspector !== false
    if (!enableInspector) {
      this.logger.info('Inspector disabled, skipping proxy server creation')
      return
    }

    // Check if hostname routing is enabled
    const useHostnameRouting = process.env['USE_HOSTNAME_ROUTING'] === 'true'
    if (useHostnameRouting) {
      // When hostname routing is enabled, the global HostnameProxyService handles all proxying
      // We don't need a per-task proxy server
      const protocol = process.env['PROXY_PROTOCOL'] || 'http'
      const proxyDomain = process.env['PROXY_DOMAIN'] || 'proxy.moatless.ai'
      this.proxyUrl = `${protocol}://${this.taskId}.${proxyDomain}`

      this.logger.info('Using hostname-based routing, skipping per-task proxy creation', {
        proxyUrl: this.proxyUrl,
        taskId: this.taskId,
      })
      return
    }

    if (!this.port) {
      this.logger.warn('Cannot start proxy without main server port')
      return
    }

    // Port-based routing: create per-task proxy server
    try {
      this.proxyPort = await getPort()
      const targetUrl = `http://localhost:${this.port}`
      const scriptContent = getPreviewInspectorScript(this.taskId)

      this.previewProxy = createPreviewProxy(
        targetUrl,
        scriptContent,
        this.logger,
        this.proxyPort,
        this.taskId,
        this.eventService,
      )

      await this.previewProxy.start()
      this.proxyUrl = this.previewProxy.getProxyUrl()

      this.logger.info('Preview proxy started (port-based)', {
        proxyPort: this.proxyPort,
        proxyUrl: this.proxyUrl,
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
        ...(request.body !== undefined && { body: request.body }),
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
   * Clean up resources after stop (keeps port allocation)
   */
  private cleanupForStop(): void {
    this.shellCommand = undefined
    // Keep this.port for reuse on restart
    this.proxyPort = undefined
    this.proxyUrl = undefined
    this.previewProxy = undefined
  }

  /**
   * Clean up all resources (full disposal)
   */
  private cleanup(): void {
    this.shellCommand = undefined
    this.port = undefined
    this.proxyPort = undefined
    this.proxyUrl = undefined
    this.previewProxy = undefined
  }

  /**
   * Dispose of the preview server completely
   */
  async dispose(): Promise<void> {
    await this.stop()
    this.cleanup()
  }
}
