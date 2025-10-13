import { tmpdir } from 'node:os'
import path from 'node:path'
import { getPort, ShellCommand } from '@moatless/shell-utils'
import { getPreviewInspectorScript } from '../core/inspector-script.js'
import type {
  Logger,
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerStatus,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
  ServerStatusType,
} from '../types/index.js'
import { createPreviewProxy, type PreviewProxyService } from './preview-proxy.js'

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
  private previewProxy: PreviewProxyService | undefined
  private proxyPort: number | undefined
  private proxyUrl: string | undefined
  private httpReadyState: 'pending' | 'ready' | 'timeout' = 'pending'

  constructor(
    private config: PreviewServerConfig,
    private logger: Logger,
    private lifecycleSink?: PreviewServerLifecycleSink,
  ) {
    // Use provided log file or undefined
    this.logFile = config.logFile

    this.logger.info('PreviewServer initialized', {
      serverId: this.config.serverId,
      workspacePath: config.workspacePath,
      hasLifecycleSink: !!this.lifecycleSink,
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
      {
        port: this.port,
        maxAttempts,
        timeoutSeconds,
      },
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
            {
              port: this.port,
              attempt: i + 1,
              maxAttempts,
              elapsedSeconds,
            },
            'Still waiting for server to start...',
          )
        } else {
          this.logger.debug(
            {
              port: this.port,
              attempt: i + 1,
              maxAttempts,
            },
            'Server not ready yet',
          )
        }
        await new Promise((resolve) => setTimeout(resolve, delayMs))
      }
    }
    return false
  }

  private async notifyStatusChange(status: ServerStatusType, error?: string): Promise<void> {
    if (!this.lifecycleSink?.onStatusChange) {
      return
    }

    const serverPid = this.shellCommand?.getPid()
    const duration = this.shellCommand?.getDuration()
    const url = this.proxyUrl || (this.port ? `http://localhost:${this.port}` : undefined)
    const uptimeSeconds = duration ? Math.floor(duration / 1000) : 0

    // Build status info object, only including properties that have values
    const statusInfo: ServerStatusChangeInfo = { status, uptimeSeconds }
    if (this.port !== undefined) {
      statusInfo.port = this.port
    }
    if (serverPid !== undefined) {
      statusInfo.pid = serverPid
    }
    if (url !== undefined) {
      statusInfo.url = url
    }
    if (error !== undefined) {
      statusInfo.error = error
    }

    try {
      await this.lifecycleSink.onStatusChange(this.config.serverId, statusInfo)
      this.logger.debug('Status change notified', {
        status,
        serverId: this.config.serverId,
      })
    } catch (callbackError) {
      this.logger.warn('Failed to notify status change via lifecycle sink', {
        serverId: this.config.serverId,
        status,
        error: callbackError instanceof Error ? callbackError.message : String(callbackError),
      })
    }
  }

  private async notifyLifecycleRegister(snapshot: PreviewServerSnapshot): Promise<void> {
    if (!this.lifecycleSink) {
      return
    }

    try {
      await this.lifecycleSink.register(this.config.serverId, snapshot)
    } catch (error) {
      this.logger.warn('Failed to register preview server in lifecycle sink', {
        serverId: this.config.serverId,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  private async notifyLifecycleUpdate(updates: PreviewServerUpdate): Promise<void> {
    if (!this.lifecycleSink) {
      return
    }

    try {
      await this.lifecycleSink.update(this.config.serverId, updates)
    } catch (error) {
      this.logger.warn('Failed to update preview server lifecycle sink', {
        serverId: this.config.serverId,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  private async notifyLifecycleRemove(): Promise<void> {
    if (!this.lifecycleSink) {
      return
    }

    try {
      await this.lifecycleSink.remove(this.config.serverId)
    } catch (error) {
      this.logger.warn('Failed to remove preview server from lifecycle sink', {
        serverId: this.config.serverId,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  /**
   * Start the preview server
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: The start sequence coordinates multiple failure states and proxy setup; breaking it up would duplicate error handling.
  async start(): Promise<PreviewServerStatus> {
    this.logger.info('Starting preview server', {
      serverCommands: this.config.serverCommands,
      envVars: this.config.envVars ? Object.keys(this.config.envVars) : [],
    })

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
    const commandsArray = this.config.serverCommands

    // Always create a new log file for each server start to avoid showing old logs
    const timestamp = Date.now()
    const logFileName = `server-${this.config.serverId}-${timestamp}.log`
    this.logFile = path.join(tmpdir(), logFileName)
    this.logger.info('Created new log file for server start', {
      logFile: this.logFile,
    })

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

    // Determine proxy URL early (before starting server) so we can use it in env vars
    await this.determineProxyUrl()

    const initialState: PreviewServerSnapshot = {
      status: 'starting',
      inspectorEnabled: this.config.enableInspector !== false,
    }

    if (this.port !== undefined) {
      initialState.port = this.port
      initialState.url = `http://localhost:${this.port}`
    }

    await this.notifyLifecycleRegister(initialState)

    try {
      // Run setup commands first if configured
      if (this.config.setupCommands && this.config.setupCommands.length > 0) {
        this.logger.info('Running setup commands before starting server', {
          setupCommands: this.config.setupCommands,
          workspacePath: this.config.workspacePath,
        })

        // Emit 'starting' status during setup
        await this.notifyStatusChange('starting')

        for (const setupCommand of this.config.setupCommands) {
          this.logger.info(`Running setup command: ${setupCommand}`)

          const setupShellCommand = new ShellCommand()
          try {
            const options: {
              workingDirectory: string
              timeout: number
              logFile: string
              env?: Record<string, string>
            } = {
              workingDirectory: this.config.workspacePath,
              timeout: 300000, // 5 minutes timeout for setup
              logFile: this.logFile,
            }

            if (this.config.envVars) {
              options.env = this.config.envVars
            }

            const setupResult = await setupShellCommand.executeWithResult(setupCommand, options)

            if (setupResult.exitCode !== 0) {
              const error = `Setup command failed: ${setupCommand}. Exit code: ${setupResult.exitCode}. Output: ${setupResult.output}`
              this.logger.error({ setupCommand, exitCode: setupResult.exitCode }, error)
              await this.notifyStatusChange('failed', error)
              await this.notifyLifecycleUpdate({ status: 'failed', error })

              return {
                status: 'failed',
                url: this.port ? `http://localhost:${this.port}` : undefined,
                pid: 0,
                uptimeSeconds: 0,
                error,
                logs: setupResult.output,
              }
            }

            this.logger.info('Setup command completed successfully', {
              setupCommand,
              exitCode: setupResult.exitCode,
              durationMs: setupResult.durationMs,
            })
          } catch (setupError) {
            const error = `Setup command failed with exception: ${setupCommand}. Error: ${setupError instanceof Error ? setupError.message : String(setupError)}`
            this.logger.error({ setupCommand }, error, setupError as Error)
            await this.notifyStatusChange('failed', error)
            await this.notifyLifecycleUpdate({ status: 'failed', error })

            return {
              status: 'failed',
              url: this.port ? `http://localhost:${this.port}` : undefined,
              pid: 0,
              uptimeSeconds: 0,
              error,
            }
          }
        }

        this.logger.info('All setup commands completed successfully')
      }

      this.shellCommand = new ShellCommand()

      // Prepare the command - replace {PORT} placeholder if present
      const command = commandsArray
        .map((cmd) => cmd.replace(/{PORT}/g, String(this.port)))
        .join(' && ')

      this.logger.info('Starting server with command', {
        command,
        workspacePath: this.config.workspacePath,
        logFile: this.logFile,
        port: this.port,
      })

      // Emit 'starting' event before starting the process
      await this.notifyStatusChange('starting')

      // Start the command in background with PORT environment variable
      await this.shellCommand.execute(command, {
        workingDirectory: this.config.workspacePath,
        env: {
          ...this.config.envVars,
          PORT: String(this.port),
        },
        background: true,
        logFile: this.logFile,
      })

      const pid = this.shellCommand.getPid()
      if (!pid) {
        const error = 'Failed to get process PID'
        this.logger.error({ command, error }, 'Failed to start server - no PID')
        await this.notifyStatusChange('failed', error)
        await this.notifyLifecycleUpdate({ status: 'failed', error })
        return {
          status: 'failed',
          url: this.port ? `http://localhost:${this.port}` : undefined,
          pid: 0,
          uptimeSeconds: 0,
          error,
        }
      }

      await this.notifyLifecycleUpdate({ pid })

      this.logger.info({ serverPid: pid, command, port: this.port }, 'Server process started')

      // Check immediate status
      const immediateStatus = this.shellCommand.getStatus()
      if (immediateStatus === 'failed') {
        const error = 'Process failed to start'
        await this.notifyLifecycleUpdate({ status: 'failed', error })
        await this.notifyStatusChange('failed', error)
        return {
          status: 'failed',
          url: this.port ? `http://localhost:${this.port}` : undefined,
          pid,
          uptimeSeconds: 0,
          error,
        }
      }

      // Start proxy if configured
      await this.startProxy()

      // Wait for server to be HTTP-responsive before emitting 'started'
      const timeoutSeconds = this.config.startupTimeoutSeconds ?? 30
      this.logger.info(
        {
          port: this.port,
          timeoutSeconds,
        },
        'Waiting for server to be HTTP-responsive...',
      )
      const isReady = await this.waitForHttpReady(timeoutSeconds)

      if (!isReady) {
        const error = 'Server process started but did not become HTTP-responsive'
        this.logger.error({ port: this.port }, error)
        this.httpReadyState = 'timeout'
        await this.notifyLifecycleUpdate({ status: 'failed', error })
        await this.notifyStatusChange('failed', error)
        return {
          status: 'failed',
          url: this.port ? `http://localhost:${this.port}` : undefined,
          pid,
          uptimeSeconds: 0,
          error,
        }
      }

      // Mark as HTTP-ready and emit started event
      this.httpReadyState = 'ready'
      const startedUpdate: PreviewServerUpdate = {
        status: 'started',
        pid,
        error: undefined,
        proxyUrl: this.proxyUrl ?? undefined,
      }

      if (this.port !== undefined) {
        startedUpdate.port = this.port
        startedUpdate.url = `http://localhost:${this.port}`
      } else {
        startedUpdate.port = undefined
        startedUpdate.url = undefined
      }

      await this.notifyLifecycleUpdate(startedUpdate)
      await this.notifyStatusChange('started')

      return {
        status: 'started',
        url: this.port ? `http://localhost:${this.port}` : undefined,
        proxyUrl: this.proxyUrl,
        pid,
        uptimeSeconds: 0,
        currentPath: this.previewProxy?.getCurrentPath(),
      }
    } catch (error) {
      this.logger.error('Failed to start server', error as Error)
      const errorMsg = error instanceof Error ? error.message : String(error)
      await this.notifyLifecycleUpdate({ status: 'failed', error: errorMsg })
      await this.notifyStatusChange('failed', errorMsg)
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
        await this.notifyStatusChange('stopped')
        this.logger.info('Server stopped successfully')
      }

      const stoppedUpdate: PreviewServerUpdate = {
        status: 'stopped',
        pid: undefined,
        proxyUrl: undefined,
        error: undefined,
      }

      if (this.port !== undefined) {
        stoppedUpdate.port = this.port
        stoppedUpdate.url = `http://localhost:${this.port}`
      } else {
        stoppedUpdate.port = undefined
        stoppedUpdate.url = undefined
      }

      await this.notifyLifecycleUpdate(stoppedUpdate)

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
  getStatus(): PreviewServerStatus {
    if (!this.shellCommand) {
      return { status: 'stopped' }
    }

    const commandStatus = this.shellCommand.getStatus()
    const exitCode = this.shellCommand.getExitCode()
    const commandError = this.shellCommand.getError()
    const duration = this.shellCommand.getDuration()

    // If not running, determine if it failed or stopped
    if (commandStatus !== 'running') {
      const isFailure = commandStatus === 'failed' || (exitCode !== undefined && exitCode !== 0)
      const error =
        commandError ||
        (isFailure && exitCode !== undefined ? `Process exited with code ${exitCode}` : undefined)

      return {
        status: isFailure ? 'failed' : 'stopped',
        error,
      }
    }

    // Process is running - check HTTP readiness state
    const uptime = duration ? Math.floor(duration / 1000) : 0
    const shellPid = this.shellCommand.getPid()

    if (this.port) {
      // Check HTTP readiness state to determine actual status
      if (this.httpReadyState === 'timeout') {
        // Process is running but failed to become HTTP-responsive
        return {
          status: 'failed',
          error:
            'Server process is running but did not become HTTP-responsive. ' +
            'This may indicate the server does not support the PORT environment variable.',
        }
      }

      if (this.httpReadyState === 'ready') {
        // Server is running and HTTP-responsive
        return {
          status: 'started',
          url: `http://localhost:${this.port}`,
          proxyUrl: this.proxyUrl,
          pid: shellPid,
          uptimeSeconds: uptime,
          currentPath: this.previewProxy?.getCurrentPath(),
        }
      }

      // httpReadyState is 'pending' - still starting
      return {
        status: 'starting',
        pid: shellPid,
        uptimeSeconds: uptime,
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
  getLogs(lines = 500): string {
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
   * Stream server logs from the log file
   * Reads existing content first, then polls for new content
   */
  async *streamLogs(): AsyncGenerator<LogStreamEvent> {
    if (!this.logFile) {
      yield {
        type: 'error',
        data: {
          error: 'No log file available',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    const { promises: fs } = await import('node:fs')
    let position = 0

    try {
      // Read existing log content first (catch up on history)
      try {
        const existingContent = await fs.readFile(this.logFile, 'utf-8')
        if (existingContent) {
          yield {
            type: 'chunk',
            data: {
              text: existingContent,
              timestamp: new Date().toISOString(),
            },
          }
          position = Buffer.byteLength(existingContent, 'utf-8')
        }
      } catch {
        // File doesn't exist yet - that's ok, we'll wait for it
        this.logger.debug('Log file not found yet, will poll for it', { logFile: this.logFile })
      }

      // Poll for new content every 100ms
      while (true) {
        try {
          const stats = await fs.stat(this.logFile)

          // Check if file has new content
          if (stats.size > position) {
            // Read new content from current position
            const fileHandle = await fs.open(this.logFile, 'r')
            const buffer = Buffer.alloc(stats.size - position)
            await fileHandle.read(buffer, 0, buffer.length, position)
            await fileHandle.close()

            const newContent = buffer.toString('utf-8')
            if (newContent) {
              yield {
                type: 'chunk',
                data: {
                  text: newContent,
                  timestamp: new Date().toISOString(),
                },
              }
              position = stats.size
            }
          }
        } catch {
          // File might not exist yet or was deleted - continue polling
        }

        // Wait 100ms before next poll
        await new Promise((resolve) => setTimeout(resolve, 100))
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
    const enableInspector = this.config.enableInspector !== false
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
      this.proxyUrl = `${protocol}://${this.config.serverId}.${proxyDomain}`

      this.logger.info('Using hostname-based routing, skipping per-task proxy creation', {
        proxyUrl: this.proxyUrl,
        serverId: this.config.serverId,
      })
      await this.notifyLifecycleUpdate({ proxyUrl: this.proxyUrl })
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
      const scriptContent = getPreviewInspectorScript(this.config.serverId)

      this.previewProxy = createPreviewProxy(
        targetUrl,
        scriptContent,
        this.logger,
        this.proxyPort,
        this.config.serverId,
      )

      await this.previewProxy.start()
      this.proxyUrl = this.previewProxy.getProxyUrl()

      this.logger.info('Preview proxy started (port-based)', {
        proxyPort: this.proxyPort,
        proxyUrl: this.proxyUrl,
        targetPort: this.port,
        targetUrl,
      })
      await this.notifyLifecycleUpdate({ proxyUrl: this.proxyUrl })
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
   * Replace placeholders in environment variable values
   * Supports {PORT} and {HOSTNAME} placeholders
   * @param envVars - Original environment variables
   * @param port - Allocated port number
   * @param proxyUrl - Proxy URL (if available)
   * @returns Environment variables with placeholders replaced
   */
  private replaceEnvVarPlaceholders(
    envVars: Record<string, string> | undefined,
    port: number,
    proxyUrl: string | undefined,
  ): Record<string, string> | undefined {
    if (!envVars) {
      return envVars
    }

    const replaced: Record<string, string> = {}

    for (const [key, value] of Object.entries(envVars)) {
      let replacedValue = value

      // Replace {PORT} placeholder
      replacedValue = replacedValue.replace(/{PORT}/g, String(port))

      // Replace {HOSTNAME} placeholder with proxyUrl or fallback to localhost
      if (replacedValue.includes('{HOSTNAME}')) {
        const hostname = proxyUrl || `http://localhost:${port}`
        replacedValue = replacedValue.replace(/{HOSTNAME}/g, hostname)
      }

      replaced[key] = replacedValue
    }

    return replaced
  }

  /**
   * Clean up resources after stop (keeps port allocation)
   */
  private cleanupForStop(): void {
    this.shellCommand = undefined
    this.httpReadyState = 'pending'
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
    this.httpReadyState = 'pending'
    this.port = undefined
    this.proxyPort = undefined
    this.proxyUrl = undefined
    this.previewProxy = undefined
    this.notifyLifecycleUpdate({
      status: 'stopped',
      port: undefined,
      url: undefined,
      proxyUrl: undefined,
      pid: undefined,
    }).catch((error) => {
      this.logger.warn('Failed to update lifecycle sink during cleanup', {
        serverId: this.config.serverId,
        error: error instanceof Error ? error.message : String(error),
      })
    })
  }

  /**
   * Dispose of the preview server completely
   */
  async dispose(): Promise<void> {
    await this.stop()
    this.cleanup()
    await this.notifyLifecycleRemove()
  }
}
