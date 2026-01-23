import { randomUUID } from 'node:crypto'
import http from 'node:http'
import { getSafeSystemEnvironment, ShellCommand } from '@moatless/shell-utils'
import { logger } from '../../../shared/logger'
import { type BrowserServer, getBrowserServer } from '../../browser'
import { loadProxyConfig, type ProxyEnvironmentConfig } from '../config/proxy-config'
import { injectScriptIntoHtml } from '../inspector/html-injector'
import { getPreviewInspectorScript } from '../inspector/inspector-script'
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
  private httpReadyState: 'pending' | 'ready' | 'timeout' = 'pending'
  private readonly logger: typeof logger
  private readonly proxyConfig: ProxyEnvironmentConfig
  private sessionId: string = randomUUID() // Track server session for log streaming
  private dirtyContext = false // Track if workspace code has been modified
  private isStopping = false // Guard against concurrent stop calls
  private isStarting = false // Guard against concurrent start calls
  private browserServer: BrowserServer | undefined // Persistent browser for automation

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
    // Port-based: point to sandbox's own /preview endpoint
    const sandboxPort = process.env['PORT'] || '8001'
    return `http://${this.proxyConfig.serverHost}:${sandboxPort}/preview`
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
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: The start sequence coordinates multiple failure states; breaking it up would duplicate error handling.
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
        const serverCommands = this.config.serverCommands.map((cmd) =>
          cmd.replace(/{PORT}/g, String(this.port)),
        )
        commands.push(...serverCommands)

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

        const safeEnv = getSafeSystemEnvironment()

        this.shellCommand = new ShellCommand({
          workingDirectory: this.config.workspacePath,
          inheritEnv: false,
          env: {
            ...safeEnv,
            ...processedEnvVars,
            PORT: String(this.port),
            NODE_ENV: processedEnvVars?.['NODE_ENV'] || 'development',
          },
        })
        const completionPromise = this.shellCommand.execute(commands)

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

        // Start browser server for automation after preview is ready
        await this.startBrowserServer()
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
      // Stop browser server first
      if (this.browserServer) {
        this.logger.debug('Stopping browser server')
        await this.browserServer.stop().catch((error) => {
          this.logger.warn({ err: error as Error }, 'Error stopping browser server')
        })
        this.browserServer = undefined
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
   * Handle a proxied preview request.
   * Uses raw HTTP pipe for non-HTML assets (preserves content-encoding).
   * Only intercepts HTML index pages for inspector script injection.
   */
  async handleProxyRequest(req: Request, path: string): Promise<Response> {
    if (!this.port) {
      return new Response('Preview server not ready', { status: 503 })
    }

    const url = `http://${this.proxyConfig.serverHost}:${this.port}${path}`
    const isIndex = path === '/' || path.endsWith('/index.html')

    // Only intercept potential HTML index pages for inspector script injection
    if (req.method === 'GET' && isIndex) {
      const requestHeaders = new Headers(req.headers)
      requestHeaders.delete('accept-encoding')
      requestHeaders.delete('host')

      const response = await fetch(url, {
        method: 'GET',
        headers: requestHeaders,
      })

      const contentType = response.headers.get('content-type') || ''
      if (contentType.includes('text/html')) {
        const html = await response.text()
        const scriptContent = getPreviewInspectorScript(this.config.serverId)
        const modified = injectScriptIntoHtml(html, scriptContent)

        const headers = new Headers()
        response.headers.forEach((v, k) => {
          if (
            ![
              'transfer-encoding',
              'connection',
              'keep-alive',
              'content-length',
              'content-encoding',
            ].includes(k.toLowerCase())
          ) {
            headers.set(k, v)
          }
        })
        headers.delete('etag')
        headers.delete('last-modified')
        headers.set('cache-control', 'no-cache, no-store, must-revalidate')
        headers.delete('x-frame-options')
        headers.delete('content-security-policy')

        return new Response(modified, { status: response.status, headers })
      }
      // Not actually HTML — fall through to raw pipe
    }

    // For all non-HTML: pipe raw bytes unchanged using node:http
    return this.rawPipe(req, url)
  }

  /**
   * Pipe request/response bytes unchanged using node:http.
   * Preserves content-encoding so the browser can decode compressed assets correctly.
   */
  private rawPipe(req: Request, targetUrl: string): Promise<Response> {
    return new Promise((resolve, reject) => {
      const parsed = new URL(targetUrl)
      const options: http.RequestOptions = {
        hostname: parsed.hostname,
        port: parsed.port,
        path: parsed.pathname + parsed.search,
        method: req.method,
        headers: { ...Object.fromEntries(req.headers.entries()), host: parsed.host },
      }

      const proxyReq = http.request(options, (proxyRes) => {
        // Convert node:http headers to web Headers
        const headers = new Headers()
        for (const [key, value] of Object.entries(proxyRes.headers)) {
          if (value != null) {
            if (Array.isArray(value)) {
              for (const v of value) {
                headers.append(key, v)
              }
            } else {
              headers.set(key, value)
            }
          }
        }

        // Stream response body as ReadableStream
        const body = new ReadableStream({
          start(controller) {
            proxyRes.on('data', (chunk: Buffer) => controller.enqueue(new Uint8Array(chunk)))
            proxyRes.on('end', () => controller.close())
            proxyRes.on('error', (err) => controller.error(err))
          },
        })

        resolve(new Response(body, { status: proxyRes.statusCode || 200, headers }))
      })

      proxyReq.on('error', (err) => {
        reject(err)
      })

      // Pipe request body if present
      if (req.body && req.method !== 'GET' && req.method !== 'HEAD') {
        const reader = req.body.getReader()
        const pump = async () => {
          while (true) {
            const { done, value } = await reader.read()
            if (done) {
              proxyReq.end()
              break
            }
            proxyReq.write(value)
          }
        }
        pump().catch(() => proxyReq.destroy())
      } else {
        proxyReq.end()
      }
    })
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
   * Start the browser server for automation
   * This runs after the preview server is HTTP-ready
   */
  private async startBrowserServer(): Promise<void> {
    try {
      this.browserServer = getBrowserServer()

      // Check if Playwright is available before trying to start
      if (!this.browserServer.isAvailable()) {
        // Initialize to check availability
        await this.browserServer.start().catch(() => {
          // Will fail if not available - that's OK
        })
      }

      if (this.browserServer.isAvailable()) {
        if (!this.browserServer.isRunning()) {
          await this.browserServer.start()
        }
        this.logger.info(
          { wsEndpoint: this.browserServer.getWsEndpoint() },
          'Browser server started for automation',
        )
      } else {
        this.logger.info(
          { reason: this.browserServer.getUnavailableReason() },
          'Browser automation not available (Playwright not installed)',
        )
        this.browserServer = undefined
      }
    } catch (error) {
      // Browser server is optional - don't fail preview if it fails
      this.logger.warn(
        { err: error as Error },
        'Failed to start browser server (automation will be unavailable)',
      )
      this.browserServer = undefined
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
  }

  /**
   * Clean up all resources (full disposal)
   */
  private cleanup(): void {
    this.shellCommand = undefined
    this.httpReadyState = 'pending'
    this.port = undefined
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
