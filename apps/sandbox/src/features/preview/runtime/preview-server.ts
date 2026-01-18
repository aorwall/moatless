import { randomUUID } from 'node:crypto'
import { getPort, ShellCommand } from '@moatless/shell-utils'
import { logger } from '../../../shared/logger'
import { loadProxyConfig, type ProxyEnvironmentConfig } from '../config/proxy-config'
import { getPreviewInspectorScript } from '../inspector/inspector-script'
import type { PreviewProxyService } from '../services/preview-proxy.service'
import { createPreviewProxy } from '../services/preview-proxy.service'
import type {
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerStatus,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
  ServerStatusType,
} from '../types/index'

interface LogStreamEvent {
  type: 'chunk' | 'error' | 'complete' | 'reset'
  data: {
    text?: string
    error?: string
    timestamp: string
  }
}

export class PreviewServer {
  private shellCommand: ShellCommand | undefined
  private port: number | undefined
  private previewProxy: PreviewProxyService | undefined
  private proxyPort: number | undefined
  private httpReadyState: 'pending' | 'ready' | 'timeout' = 'pending'
  private readonly logger: typeof logger
  private readonly proxyConfig: ProxyEnvironmentConfig
  private sessionId: string = randomUUID() // Track server session for log streaming
  private dirtyContext = false // Track if workspace code has been modified
  private isStopping = false // Guard against concurrent stop calls
  private isStarting = false // Guard against concurrent start calls

  constructor(
    private config: PreviewServerConfig,
    private lifecycleSink?: PreviewServerLifecycleSink,
  ) {
    // Create child logger with server context
    this.logger = logger.child({ service: 'preview-server', serverId: config.serverId })

    // Validate configuration
    if (!config.serverCommands || config.serverCommands.length === 0) {
      throw new Error(
        'Preview server configuration requires at least one server start command. ' +
          'Please configure serverStartCommands in your repository preview settings.',
      )
    }

    // Load proxy configuration once
    this.proxyConfig = loadProxyConfig()

    this.logger.info(
      {
        serverId: this.config.serverId,
        workspacePath: config.workspacePath,
        hasLifecycleSink: !!this.lifecycleSink,
        hasSetupCommands: (this.config.setupCommands?.length ?? 0) > 0,
        proxyConfig: {
          useHostnameRouting: this.proxyConfig.useHostnameRouting,
          serverHost: this.proxyConfig.serverHost,
        },
      },
      'PreviewServer initialized',
    )
  }

  /**
   * Try to connect to a single URL for health check
   * @returns Object with success flag and error message if failed
   */
  private async tryHealthCheckUrl(url: string): Promise<{ success: boolean; error?: string }> {
    try {
      const response = await fetch(url, {
        signal: AbortSignal.timeout(3000),
      })
      if (response.ok) {
        return { success: true }
      }
      return { success: false, error: `HTTP ${response.status}` }
    } catch (error) {
      return {
        success: false,
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Check port using netcat as a fallback diagnostic
   * @returns Object with listening status and diagnostic info
   */
  private async checkPortWithNetcat(
    port: number,
  ): Promise<{ listening: boolean; diagnostic: string }> {
    const { spawn } = await import('node:child_process')
    return new Promise((resolve) => {
      try {
        const proc = spawn('nc', ['-z', '-v', 'localhost', String(port)], {
          stdio: ['ignore', 'pipe', 'pipe'],
        })
        let stderr = ''
        proc.stderr?.on('data', (data: Buffer) => {
          stderr += data.toString()
        })
        proc.on('close', (code) => {
          resolve({
            listening: code === 0,
            diagnostic: stderr.trim() || `exit code ${code}`,
          })
        })
        proc.on('error', (err) => {
          resolve({
            listening: false,
            diagnostic: `nc spawn error: ${err.message}`,
          })
        })
        // Timeout after 2 seconds
        setTimeout(() => {
          proc.kill()
          resolve({ listening: false, diagnostic: 'nc timeout' })
        }, 2000)
      } catch (error) {
        resolve({
          listening: false,
          diagnostic: `nc failed: ${error instanceof Error ? error.message : String(error)}`,
        })
      }
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

    const delayMs = 1000
    const maxAttempts = Math.floor(timeoutSeconds / (delayMs / 1000))

    this.logger.info(
      { port: this.port, maxAttempts, timeoutSeconds },
      'Waiting for server to become HTTP-responsive',
    )

    let lastError: string | undefined
    // Try IPv6, localhost, and IPv4 to handle different address binding behaviors
    // Note: Vite in Docker may bind to IPv6 (::1) by default
    const urlsToTry = [
      `http://[::1]:${this.port}`, // IPv6 loopback (vite default in Docker)
      `http://localhost:${this.port}`, // May resolve to IPv6 or IPv4
      `http://127.0.0.1:${this.port}`, // IPv4 loopback
    ]

    for (let i = 0; i < maxAttempts; i++) {
      if (this.isStopping) {
        this.logger.info({ port: this.port }, 'Server is stopping, exiting health check')
        return false
      }

      // Check process status every 10 attempts
      if ((i + 1) % 10 === 0) {
        const processStatus = this.shellCommand?.getStatus()
        if (processStatus && processStatus !== 'running') {
          this.logger.warn(
            { port: this.port, attempt: i + 1, processStatus },
            'Process exited during health check',
          )
          lastError = `Process exited (status: ${processStatus})`
          break
        }
      }

      // Try each URL
      for (const url of urlsToTry) {
        const result = await this.tryHealthCheckUrl(url)
        if (result.success) {
          this.logger.info({ port: this.port, attempts: i + 1, url }, 'Server is HTTP-responsive')
          return true
        }
        lastError = `${url}: ${result.error}`
      }

      // Log periodically for visibility with diagnostic
      if (i === 0 || (i + 1) % 10 === 0) {
        // Run netcat diagnostic to check if port is actually listening
        const ncResult = await this.checkPortWithNetcat(this.port!)
        this.logger.info(
          {
            port: this.port,
            attempt: i + 1,
            maxAttempts,
            error: lastError,
            ncListening: ncResult.listening,
            ncDiagnostic: ncResult.diagnostic,
          },
          'Health check: connection failed',
        )
      }

      await new Promise((resolve) => setTimeout(resolve, delayMs))
    }

    this.logger.error(
      {
        port: this.port,
        maxAttempts,
        lastError,
        finalProcessStatus: this.shellCommand?.getStatus(),
      },
      'Health check failed after all attempts',
    )
    return false
  }

  /**
   * Derive the proxy URL based on current state
   * @returns The proxy URL if configured, undefined otherwise
   */
  private getProxyUrl(): string | undefined {
    if (this.proxyConfig.useHostnameRouting && this.proxyConfig.proxyDomain) {
      return `${this.proxyConfig.proxyProtocol}://${this.config.serverId}.${this.proxyConfig.proxyDomain}`
    }
    if (this.proxyPort) {
      return `http://${this.proxyConfig.serverHost}:${this.proxyPort}`
    }
    return undefined
  }

  private async notifyStatusChange(status: ServerStatusType, error?: string): Promise<void> {
    if (!this.lifecycleSink?.onStatusChange) {
      return
    }

    const serverPid = this.shellCommand?.getPid()
    const duration = this.shellCommand?.getDuration()
    const externalUrl = this.getProxyUrl()
    const internalUrl = this.port ? `http://${this.proxyConfig.serverHost}:${this.port}` : undefined
    const uptimeSeconds = duration ? Math.floor(duration / 1000) : 0

    // Build status info object, only including properties that have values
    const statusInfo: ServerStatusChangeInfo = { status, uptimeSeconds }
    if (this.port !== undefined) {
      statusInfo.port = this.port
    }
    if (serverPid !== undefined) {
      statusInfo.pid = serverPid
    }
    if (internalUrl !== undefined) {
      statusInfo.internalUrl = internalUrl
    }
    if (externalUrl !== undefined) {
      statusInfo.externalUrl = externalUrl
    }
    if (error !== undefined) {
      statusInfo.error = error
    }

    try {
      await this.lifecycleSink.onStatusChange(this.config.serverId, statusInfo)
      this.logger.debug(
        {
          status,
          serverId: this.config.serverId,
        },
        'Status change notified',
      )
    } catch (callbackError) {
      this.logger.warn(
        {
          serverId: this.config.serverId,
          status,
          error: callbackError instanceof Error ? callbackError.message : String(callbackError),
        },
        'Failed to notify status change via lifecycle sink',
      )
    }
  }

  private async notifyLifecycleRegister(snapshot: PreviewServerSnapshot): Promise<void> {
    if (!this.lifecycleSink) {
      return
    }

    try {
      await this.lifecycleSink.register(this.config.serverId, snapshot)
    } catch (error) {
      this.logger.warn(
        {
          serverId: this.config.serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to register preview server in lifecycle sink',
      )
    }
  }

  private async notifyLifecycleUpdate(updates: PreviewServerUpdate): Promise<void> {
    if (!this.lifecycleSink) {
      return
    }

    try {
      await this.lifecycleSink.update(this.config.serverId, updates)
    } catch (error) {
      this.logger.warn(
        {
          serverId: this.config.serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to update preview server lifecycle sink',
      )
    }
  }

  private async notifyLifecycleRemove(): Promise<void> {
    if (!this.lifecycleSink) {
      return
    }

    try {
      await this.lifecycleSink.remove(this.config.serverId)
    } catch (error) {
      this.logger.warn(
        {
          serverId: this.config.serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to remove preview server from lifecycle sink',
      )
    }
  }

  /**
   * Start the preview server
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: The start sequence coordinates multiple failure states and proxy setup; breaking it up would duplicate error handling.
  async start(): Promise<void> {
    // Concurrency guard - prevent multiple simultaneous start calls
    if (this.isStarting) {
      this.logger.info('Start already in progress, skipping duplicate call')
      return
    }

    this.isStarting = true

    try {
      // Generate new session ID for this server start
      this.sessionId = randomUUID()

      // Clear dirty flag on server start
      this.clearDirty()

      this.logger.info(
        {
          serverCommands: this.config.serverCommands,
          setupCommands: this.config.setupCommands,
          envVars: this.config.envVars ? Object.keys(this.config.envVars) : [],
          sessionId: this.sessionId,
        },
        'Starting preview server',
      )

      // Clean up any existing command
      if (this.shellCommand?.isRunning()) {
        await this.stop()
        this.cleanup() // Only do full cleanup if we had to stop a running server
      } else if (this.shellCommand) {
        // If we have a shell command but it's not running (stopped server),
        // just clean up the command reference but keep the port
        this.shellCommand = undefined
      }

      // Only allocate a new port if we don't have one already
      if (!this.port) {
        // In sandbox mode, use fixed port 3000 for predictability
        this.port = 3000
        this.logger.info({ port: this.port }, 'Using fixed port for sandbox mode')
      } else {
        this.logger.info({ port: this.port }, 'Reusing existing port for server')
      }

      // Allocate proxy port early (before starting server) so we can use proxy URL in env vars
      await this.allocateProxyPort()

      const initialState: PreviewServerSnapshot = {
        status: 'starting',
        inspectorEnabled: this.config.enableInspector !== false,
      }

      if (this.port !== undefined) {
        initialState.port = this.port
        initialState.internalUrl = `http://${this.proxyConfig.serverHost}:${this.port}`
      }

      await this.notifyLifecycleRegister(initialState)

      try {
        // Build command sequence: setup + server start
        const commands: string[] = []

        // Add setup commands if provided (first start only)
        if (this.config.setupCommands && this.config.setupCommands.length > 0) {
          this.logger.debug(
            {
              setupCommands: this.config.setupCommands,
            },
            'Including setup commands in sequence',
          )
          commands.push(...this.config.setupCommands)
        }

        // Add server start command(s) with {PORT} placeholder replacement
        const serverCommamds = this.config.serverCommands.map((cmd) =>
          cmd.replace(/{PORT}/g, String(this.port)),
        )
        commands.push(...serverCommamds)

        this.logger.info(
          {
            commands,
            hasSetup: (this.config.setupCommands?.length ?? 0) > 0,
            workspacePath: this.config.workspacePath,
            port: this.port,
          },
          'Executing commands',
        )

        // Emit 'starting' event before starting the process
        await this.notifyStatusChange('starting')

        // Process environment variables to replace {PORT} and {HOSTNAME} placeholders
        const processedEnvVars = this.replaceEnvVarPlaceholders(
          this.config.envVars,
          this.port,
          this.getProxyUrl(),
        )

        // Defensive check: ensure we have commands to execute
        if (commands.length === 0) {
          const error =
            'No commands to execute. ' +
            `serverCommands: ${JSON.stringify(this.config.serverCommands)}, ` +
            `setupCommands: ${JSON.stringify(this.config.setupCommands)}`
          this.logger.error({ port: this.port }, error)
          await this.notifyLifecycleUpdate({ status: 'failed', error })
          await this.notifyStatusChange('failed', error)
          throw new Error(error)
        }

        this.shellCommand = new ShellCommand({
          workingDirectory: this.config.workspacePath,
          env: {
            ...processedEnvVars,
            PORT: String(this.port),
          },
        })
        const completionPromise = this.shellCommand.execute(commands)

        // Start proxy if configured
        await this.startProxy()

        // Enforce minimum of 120 seconds to handle slow CI environments and cold starts
        const configuredTimeout = this.config.startupTimeoutSeconds ?? 90
        const timeoutSeconds = Math.max(configuredTimeout, 120)
        this.logger.info(
          {
            port: this.port,
            timeoutSeconds,
          },
          'Waiting for HTTP ready or command completion...',
        )

        const httpReady = this.waitForHttpReady(timeoutSeconds)

        const result = await Promise.race([
          completionPromise.then((completion) => ({ type: 'command' as const, completion })),
          httpReady.then((ready) => ({ type: 'http' as const, ready })),
        ])

        // Handle race result
        if (result.type === 'command') {
          // Command completed (failed or exited) before HTTP ready
          const { completion } = result
          if (completion.status === 'failed') {
            const error = completion.error || 'Process failed to start'
            const output = this.shellCommand?.getOutputOnly() || ''
            this.logger.error(
              {
                port: this.port,
                exitCode: completion.exitCode,
                output: output.slice(-1000), // Last 1000 chars of output for debugging
              },
              error,
            )
            await this.notifyLifecycleUpdate({ status: 'failed', error })
            await this.notifyStatusChange('failed', error)
            return
          }
          if (completion.status === 'completed') {
            // Process exited before HTTP ready
            // Exit code 0 might indicate background fork (e.g., Vite, Next.js)
            // Give it a moment to check if server is actually responsive
            if (completion.exitCode === 0) {
              this.logger.info(
                { port: this.port, exitCode: 0 },
                'Setup process exited successfully, checking if server is responsive...',
              )
              // Quick check if server is already responsive (background fork)
              const isReady = await this.waitForHttpReady(5) // 5 second grace period
              if (isReady) {
                this.logger.info({ port: this.port }, 'Server is responsive after background fork')
                // Continue to mark as started below
              } else {
                const error =
                  'Setup process completed successfully but server is not HTTP-responsive'
                this.logger.error({ port: this.port, exitCode: 0 }, error)
                await this.notifyLifecycleUpdate({ status: 'failed', error })
                await this.notifyStatusChange('failed', error)
                return
              }
            } else {
              // Non-zero exit code without failure status is unexpected
              const error = `Process exited with code ${completion.exitCode} before becoming HTTP-responsive`
              this.logger.error({ port: this.port, exitCode: completion.exitCode }, error)
              await this.notifyLifecycleUpdate({ status: 'failed', error })
              await this.notifyStatusChange('failed', error)
              return
            }
          }
        } else if (!result.ready) {
          // HTTP ready won the race but server not ready
          const error = 'Server process started but did not become HTTP-responsive'
          const output = this.shellCommand?.getOutputOnly() || ''
          this.logger.error(
            {
              port: this.port,
              output: output.slice(-2000), // Last 2000 chars of output for debugging
            },
            error,
          )
          this.httpReadyState = 'timeout'
          await this.notifyLifecycleUpdate({ status: 'failed', error })
          await this.notifyStatusChange('failed', error)
          return
        }

        // Mark as HTTP-ready and emit started event
        this.httpReadyState = 'ready'
        const pid = this.shellCommand?.getPid()
        const startedUpdate: PreviewServerUpdate = {
          status: 'started',
          pid,
          error: undefined,
          externalUrl: this.getProxyUrl() ?? undefined,
        }

        if (this.port !== undefined) {
          startedUpdate.port = this.port
          startedUpdate.internalUrl = `http://${this.proxyConfig.serverHost}:${this.port}`
        } else {
          startedUpdate.port = undefined
          startedUpdate.internalUrl = undefined
        }

        await this.notifyLifecycleUpdate(startedUpdate)
        await this.notifyStatusChange('started')
      } catch (error) {
        this.logger.error({ err: error as Error }, 'Failed to start server')
        const errorMsg = error instanceof Error ? error.message : String(error)
        await this.notifyLifecycleUpdate({ status: 'failed', error: errorMsg })
        await this.notifyStatusChange('failed', errorMsg)
        throw error
      }
    } finally {
      this.isStarting = false
    }
  }

  /**
   * Stop the preview server
   */
  async stop(): Promise<boolean> {
    // Concurrency guard - prevent multiple simultaneous stop calls
    if (this.isStopping) {
      this.logger.info('Stop already in progress, skipping duplicate call')
      return false
    }

    // Set isStopping immediately to signal any health check loops to exit
    this.isStopping = true

    if (!this.shellCommand) {
      this.logger.info('Preview server already stopped (no shell command)')
      return false
    }

    const status = this.shellCommand.getStatus()
    if (status !== 'running') {
      this.logger.info({ status }, 'Preview server already stopped')
      return false
    }

    this.logger.info(
      {
        serverPid: this.shellCommand?.getPid(),
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

      // Get PID before killing
      const pid = this.shellCommand.getPid()

      // Kill the server process
      const killed = await this.shellCommand.kill()

      if (killed && pid) {
        // Wait for process to actually terminate
        await this.waitForProcessTermination(pid, 5000)
        await this.notifyStatusChange('stopped')
        this.logger.info('Server stopped successfully')
      }

      const stoppedUpdate: PreviewServerUpdate = {
        status: 'stopped',
        pid: undefined,
        externalUrl: undefined,
        error: undefined,
      }

      if (this.port !== undefined) {
        stoppedUpdate.port = this.port
        stoppedUpdate.internalUrl = `http://${this.proxyConfig.serverHost}:${this.port}`
      } else {
        stoppedUpdate.port = undefined
        stoppedUpdate.internalUrl = undefined
      }

      await this.notifyLifecycleUpdate(stoppedUpdate)

      return killed
    } finally {
      // Don't release the main port on stop - we want to reuse it on restart
      // Clean up everything except the main port
      this.cleanupForStop()
      this.isStopping = false
    }
  }

  /**
   * Wait for process to fully terminate
   * Uses process.kill(pid, 0) to check if process still exists
   */
  private async waitForProcessTermination(pid: number, timeoutMs: number): Promise<void> {
    const startTime = Date.now()
    while (Date.now() - startTime < timeoutMs) {
      try {
        // Signal 0 checks if process exists without actually killing it
        process.kill(pid, 0)
        // Process still exists, wait a bit
        await new Promise((resolve) => setTimeout(resolve, 100))
      } catch {
        // Process doesn't exist anymore (ESRCH error)
        this.logger.debug({ pid }, 'Process terminated successfully')
        return
      }
    }
    this.logger.warn({ pid, timeoutMs }, 'Process termination verification timed out')
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
          internalUrl: `http://${this.proxyConfig.serverHost}:${this.port}`,
          externalUrl: this.getProxyUrl(),
          pid: shellPid,
          uptimeSeconds: uptime,
          currentPath: this.previewProxy?.getCurrentPath(),
          dirtyContext: this.dirtyContext,
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
      // Get logs from ShellCommand's in-memory buffer
      const logEvents = this.shellCommand.getLogs(lines)
      return logEvents.map((log) => `[${log.timestamp}] [${log.type}] ${log.text}`).join('')
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Error reading logs:')
      return `Error reading logs: ${error}`
    }
  }

  /**
   * Stream server logs from ShellCommand's in-memory buffer
   * Yields reset event on session start, then delegates to ShellCommand.streamLogs()
   */
  async *streamLogs(): AsyncGenerator<LogStreamEvent> {
    // Capture current session ID to detect session changes
    const currentSession = this.sessionId

    // Always yield reset event first to signal start of this log session
    yield {
      type: 'reset',
      data: {
        timestamp: new Date().toISOString(),
      },
    }

    if (!this.shellCommand) {
      // No server running - wait for it to start
      // Keep connection alive by polling until server starts or session changes
      while (!this.shellCommand && this.sessionId === currentSession) {
        await new Promise((resolve) => setTimeout(resolve, 1000))
      }

      // If session changed, exit - new stream will be established
      if (this.sessionId !== currentSession) {
        return
      }
    }

    // Stream from ShellCommand's in-memory log buffer
    // ShellCommand.streamLogs() yields existing logs first, then polls for new ones
    if (this.shellCommand) {
      for await (const logEvent of this.shellCommand.streamLogs()) {
        // Stop streaming if session changed (server restarted)
        if (this.sessionId !== currentSession) {
          break
        }

        yield {
          type: 'chunk',
          data: {
            text: logEvent.text,
            timestamp: logEvent.timestamp,
          },
        }
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
   * Mark the server's workspace context as dirty (code changes detected)
   */
  markDirty(): void {
    if (this.dirtyContext) {
      return // Already dirty, no need to update
    }

    this.dirtyContext = true
    this.logger.info(
      {
        serverId: this.config.serverId,
      },
      'Workspace context marked as dirty',
    )

    // Notify lifecycle sink about dirty state change
    this.notifyLifecycleUpdate({ dirtyContext: true }).catch((error) => {
      this.logger.warn(
        {
          serverId: this.config.serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to notify lifecycle sink about dirty context',
      )
    })
  }

  /**
   * Clear the dirty context flag (after restart or when changes are applied)
   */
  clearDirty(): void {
    if (!this.dirtyContext) {
      return // Already clean, no need to update
    }

    this.dirtyContext = false
    this.logger.info(
      {
        serverId: this.config.serverId,
      },
      'Workspace context marked as clean',
    )

    // Notify lifecycle sink about clean state
    this.notifyLifecycleUpdate({ dirtyContext: false }).catch((error) => {
      this.logger.warn(
        {
          serverId: this.config.serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to notify lifecycle sink about clean context',
      )
    })
  }

  /**
   * Check if the workspace context is dirty
   */
  isDirty(): boolean {
    return this.dirtyContext
  }

  /**
   * Allocate proxy port if needed (for port-based routing)
   * This allows us to use {HOSTNAME} placeholder in environment variables
   */
  private async allocateProxyPort(): Promise<void> {
    if (this.proxyConfig.useHostnameRouting) {
      // Hostname routing doesn't need a per-task proxy port
      this.logger.info(
        {
          proxyUrl: this.getProxyUrl(),
          serverId: this.config.serverId,
        },
        'Using hostname-based routing',
      )
      return
    }

    const enableInspector = this.config.enableInspector !== false
    if (!enableInspector) {
      this.logger.info('Inspector disabled, skipping proxy port allocation')
      return
    }

    // Port-based routing: allocate proxy port
    this.proxyPort = await getPort()

    this.logger.info(
      {
        proxyPort: this.proxyPort,
        proxyUrl: this.getProxyUrl(),
      },
      'Allocated port-based proxy port',
    )
  }

  /**
   * Start the preview proxy server with inspector script injection
   * Note: When hostname routing is enabled, this is skipped - the global HostnameProxyService handles it
   * The proxy URL should already be determined by determineProxyUrl()
   */
  private async startProxy(): Promise<void> {
    const enableInspector = this.config.enableInspector !== false
    if (!enableInspector) {
      this.logger.info('Inspector disabled, skipping proxy server creation')
      return
    }

    if (this.proxyConfig.useHostnameRouting) {
      // The global HostnameProxyService handles all proxying
      this.logger.info(
        {
          externalUrl: this.getProxyUrl(),
          serverId: this.config.serverId,
        },
        'Using hostname-based routing, skipping per-task proxy creation',
      )
      await this.notifyLifecycleUpdate({ externalUrl: this.getProxyUrl() })
      return
    }

    if (!(this.port && this.proxyPort)) {
      this.logger.warn('Cannot start proxy without main server port and proxy port')
      return
    }

    // Port-based routing: create and start per-task proxy server
    // Proxy port and URL already determined in determineProxyUrl()
    try {
      const targetUrl = `http://${this.proxyConfig.serverHost}:${this.port}`
      const scriptContent = getPreviewInspectorScript(this.config.serverId)

      this.previewProxy = createPreviewProxy(
        targetUrl,
        scriptContent,
        this.proxyPort,
        this.config.serverId,
        this.proxyConfig.serverHost,
      )

      await this.previewProxy.start()

      this.logger.info(
        {
          proxyPort: this.proxyPort,
          externalUrl: this.getProxyUrl(),
          targetPort: this.port,
          targetUrl,
        },
        'Preview proxy started (port-based)',
      )
      await this.notifyLifecycleUpdate({ externalUrl: this.getProxyUrl() })
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Failed to start preview proxy')
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

    const url = `http://${this.proxyConfig.serverHost}:${this.port}${request.path}`

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
      this.logger.error(
        { err: error as Error, url, method: request.method },
        'Failed to proxy request',
      )
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
    const replacements: Record<string, { original: string; replaced: string }> = {}

    for (const [key, value] of Object.entries(envVars)) {
      let replacedValue = value
      let hasReplacement = false

      // Replace {PORT} placeholder
      if (replacedValue.includes('{PORT}')) {
        replacedValue = replacedValue.replace(/{PORT}/g, String(port))
        hasReplacement = true
      }

      // Replace {HOSTNAME} placeholder with proxyUrl or fallback to server host
      if (replacedValue.includes('{HOSTNAME}')) {
        const hostname = proxyUrl || `http://${this.proxyConfig.serverHost}:${port}`
        replacedValue = replacedValue.replace(/{HOSTNAME}/g, hostname)
        hasReplacement = true
      }

      replaced[key] = replacedValue

      // Track replacements for logging
      if (hasReplacement) {
        replacements[key] = { original: value, replaced: replacedValue }
      }
    }

    // Log all replacements at INFO level
    if (Object.keys(replacements).length > 0) {
      this.logger.info(
        {
          port,
          proxyUrl,
          replacements,
        },
        'Replaced environment variable placeholders',
      )
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
    this.previewProxy = undefined
    this.dirtyContext = false
    this.notifyLifecycleUpdate({
      status: 'stopped',
      port: undefined,
      internalUrl: undefined,
      externalUrl: undefined,
      pid: undefined,
      dirtyContext: false,
    }).catch((error) => {
      this.logger.warn(
        {
          serverId: this.config.serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to update lifecycle sink during cleanup',
      )
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
