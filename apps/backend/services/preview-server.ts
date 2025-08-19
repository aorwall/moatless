import { type ChildProcess, spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import type { RepositoryConfig, ServerStatus, ServerStatusEnum } from '@moatless/types'
import type { Logger } from 'pino'
import { pathExists, readFileRange } from '../utils/filesystem.js'
import { execAsync } from '../utils/git-utils.js'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
import { findHttpPortForPid } from '../utils/port-detection.js'
import { getPreviewInspectorScript } from '../utils/preview-inspector-script.js'
import type { EventService } from './event.service.js'
import { PortService } from './port.service.js'
import { createPreviewProxy, type PreviewProxyService } from './preview-proxy.service.js'

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
  private process?: ChildProcess
  private port?: number
  private logFile?: string
  private startTime?: number
  private failureReason?: string
  private logger = createTaskLogger('preview-server')
  private portService = new PortService(this.logger)
  private currentStatus: ServerStatusEnum = 'stopped'
  private previewProxy?: PreviewProxyService
  private proxyPort?: number
  private proxyUrl?: string
  private isIntentionallyStopping = false

  // Runtime configuration - set when start() is called
  private serverCommands?: string | string[]
  private envVars?: Record<string, string>
  private repositoryConfig?: RepositoryConfig

  constructor(
    private taskId: string,
    private workspacePath: string,
    private eventService?: EventService,
  ) {
    this.logger.info('PreviewServer initialized with:', {
      taskId: this.taskId,
      workspacePath,
      hasEventService: !!this.eventService,
    })
  }

  private async emitServerEvent(status: ServerStatusEnum, error?: string): Promise<void> {
    if (!this.eventService) {
      return
    }

    const previousStatus = this.currentStatus
    this.currentStatus = status

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
          previousStatus,
          port: this.port,
          pid: this.process?.pid,
          url: this.port ? `http://localhost:${this.port}` : undefined,
          error,
          uptimeSeconds: this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0,
        },
      })

      this.logger.debug('Server event emitted', {
        status,
        previousStatus,
        taskId: this.taskId,
      })
    } catch (eventError) {
      this.logger.error('Failed to emit server event', eventError as Error, {
        status,
        previousStatus,
        taskId: this.taskId,
      })
    }
  }

  async start(
    serverCommands: string | string[],
    envVars?: Record<string, string>,
    repositoryConfig?: RepositoryConfig,
    logFile?: string,
  ): Promise<ServerStartResult> {
    // Store configuration for potential restart
    this.serverCommands = serverCommands
    this.envVars = envVars || {}
    this.repositoryConfig = repositoryConfig

    if (this.process && this.process.exitCode === null) {
      throw new Error('Server is already running')
    }

    if (this.process && this.process.exitCode) {
      this.logger.warn('Server process exited unexpectedly, cleanup and start again', {})
      this.cleanup()
    }

    // Check if we already have a process starting up
    if (this.process && this.process.exitCode === null && !this.port) {
      // Return starting state if process exists but no port detected yet
      return {
        status: 'starting',
        port: 0,
        pid: this.process.pid || 0,
        log_file: this.logFile || '',
        uptime_seconds: 0,
      }
    }

    // Clean up any previous state
    this.cleanup()

    // Reset the stopping flag
    this.isIntentionallyStopping = false

    // Normalize commands to array and filter out empty commands
    const rawCommands = Array.isArray(this.serverCommands)
      ? this.serverCommands
      : [this.serverCommands]
    const commandsArray = rawCommands.filter((cmd) => cmd.trim() !== '')

    if (commandsArray.length === 0) {
      throw new Error('No valid server commands provided (all commands are empty)')
    }

    // Setup log file first
    if (logFile) {
      this.logFile = logFile
    } else {
      const tempDir = path.join(tmpdir(), 'swarm_server_logs')
      await fs.mkdir(tempDir, { recursive: true })
      this.logFile = path.join(tempDir, `server_${Date.now()}.log`)
    }

    // Emit initializing event
    await this.emitServerEvent('starting')

    // Prepare command and spawn process to get PID immediately
    try {
      const { command, commandOptions } = await this.prepareServerCommand(commandsArray)

      // Write command info to log file FIRST
      await fs.writeFile(
        this.logFile!,
        '=== Preview Server Log ===\n' +
          `Timestamp: ${new Date().toISOString()}\n` +
          `Working Directory: ${this.workspacePath}\n` +
          `Command: ${command}\n` +
          `Environment Variables:\n${JSON.stringify(commandOptions.env, null, 2)}\n` +
          '===========================\n\n',
      )

      // Open log stream for appending
      const logStream = await fs.open(this.logFile!, 'a')

      // Spawn the process using our own logic
      this.logger.info('Spawning server process', { command })
      const { child, timeoutHandle } = this.spawnServerCommand(command, commandOptions)

      // Store process immediately
      this.process = child
      this.startTime = Date.now()

      // Check if spawn was successful (PID validation)
      if (!this.process.pid || this.process.pid <= 0) {
        this.failureReason = 'Failed to spawn process - no valid PID assigned'
        this.logger.error('Spawn failed - no valid PID', {
          pid: this.process.pid,
          command: command,
        })
        await this.emitServerEvent('failed', this.failureReason)
        await logStream.close()

        return {
          status: 'failed',
          port: 0,
          pid: 0,
          log_file: this.logFile,
          uptime_seconds: 0,
          error: this.failureReason,
        }
      }

      this.logger.info('Process spawned successfully with PID', {
        pid: this.process.pid,
        command: command,
      })

      // IMMEDIATELY attach log handlers to capture ALL output
      let portDetected = false
      let lastPortCheck = 0
      const PORT_CHECK_INTERVAL = 1000

      if (this.process.stdout) {
        this.process.stdout.on('data', async (chunk) => {
          const text = chunk.toString()
          try {
            await logStream.write(text)
          } catch (err) {
            this.logger.warn('Failed to write stdout to log', err)
          }

          // Check for port detection
          if (!portDetected && this.process?.pid) {
            const now = Date.now()
            if (now - lastPortCheck > PORT_CHECK_INTERVAL) {
              lastPortCheck = now
              const port = await findHttpPortForPid(this.process.pid)
              if (port) {
                this.port = port
                portDetected = true
                await this.startProxy()
                await this.emitServerEvent('started')
                this.logger.info(
                  `Detected server running on port ${port} via PID ${this.process.pid}`,
                )
              }
            }
          }
        })
      }

      if (this.process.stderr) {
        this.process.stderr.on('data', async (chunk) => {
          const text = chunk.toString()
          try {
            await logStream.write(text)
          } catch (err) {
            this.logger.warn('Failed to write stderr to log', err)
          }
        })
      }

      // Monitor for process exit
      this.process.on('exit', async (code) => {
        clearTimeout(timeoutHandle)
        await logStream.close()

        if (!portDetected && !this.isIntentionallyStopping) {
          const errorMsg = `Process exited without starting a server (exit code: ${code})`
          this.logger.error('Server failed - no port detected', { exitCode: code })
          this.failureReason = errorMsg
          await this.emitServerEvent('failed', this.failureReason)
        } else if (code === 0 || this.isIntentionallyStopping) {
          this.logger.info('Server command completed successfully')
        } else {
          const errorMsg = `Command exited with code ${code}`
          this.logger.error('Server command failed', { error: errorMsg })
          this.failureReason = errorMsg
          await this.emitServerEvent('failed', this.failureReason)
        }
      })

      // Handle spawn errors
      this.process.on('error', async (error: Error) => {
        this.logger.error('Process error', error)
        this.failureReason = error.message
        await this.emitServerEvent('failed', this.failureReason)
        await logStream.close()
      })

      // Check for immediate process exit
      if (this.process.exitCode !== null) {
        const exitCode = this.process.exitCode
        this.logger.info(`Process exited immediately with code ${exitCode}`)

        if (exitCode !== 0) {
          this.failureReason = `Process exited immediately with code ${exitCode}`
          await this.emitServerEvent('failed', this.failureReason)
        }

        await logStream.close()
        return {
          status: 'failed',
          port: 0,
          pid: this.process?.pid || 0,
          log_file: this.logFile,
          uptime_seconds: 0,
          error: this.failureReason,
        }
      }

      // Return immediately with PID available
      return {
        status: 'starting',
        port: this.port || 0,
        pid: this.process?.pid || 0,
        log_file: this.logFile,
        uptime_seconds: 0,
      }
    } catch (error) {
      this.logger.error('Failed to spawn server process', error as Error)
      this.failureReason = error instanceof Error ? error.message : String(error)
      await this.emitServerEvent('failed', this.failureReason)
      throw error
    }
  }

  private async prepareServerCommand(commandsArray: string[]): Promise<{
    command: string
    commandOptions: { cwd: string; env: Record<string, string>; timeout: number }
  }> {
    this.logger.info('Preparing server command')

    // Combine all commands into a single execution context to preserve environment
    const preparationCommands = commandsArray.slice(0, -1)
    const serverCommand = commandsArray[commandsArray.length - 1]

    // Build combined command string
    let combinedCommand: string
    if (preparationCommands.length > 0) {
      this.logger.info(
        `Combining ${preparationCommands.length} preparation commands with server command for environment persistence`,
      )

      // Join all commands with && to ensure they run in the same shell context
      const allCommands = [...preparationCommands, serverCommand]
      combinedCommand = allCommands.join(' && ')

      this.logger.info(`Combined command: ${combinedCommand}`)
    } else {
      combinedCommand = serverCommand
    }

    // Handle port placeholder replacement
    const command = combinedCommand

    this.logger.info(`Final server command: ${command}`)
    this.logger.info(`Server logs will be saved to: ${this.logFile}`)

    // Ensure node_modules/.bin is in PATH for the workspace
    const workspaceBinPath = path.join(this.workspacePath, 'node_modules', '.bin')
    const currentPath = process.env.PATH || ''
    const enhancedPath = workspaceBinPath + path.delimiter + currentPath

    // Build environment with sensible defaults that can be overridden
    const defaultEnv: Record<string, string> = {
      PATH: enhancedPath,
      // Only add NODE_OPTIONS if not already provided
      NODE_OPTIONS: '--preserve-symlinks --preserve-symlinks-main',
    }

    // Merge envVars, allowing repository config to override defaults
    const mergedEnv: Record<string, string> = {
      ...defaultEnv,
      ...this.envVars, // Repository-specific envVars override defaults
    }

    const commandOptions = {
      cwd: this.workspacePath,
      env: mergedEnv,
      timeout: 300000, // 5 minute timeout for server startup
    }

    return { command, commandOptions }
  }

  /**
   * Spawn a server command with proper environment setup
   */
  private spawnServerCommand(
    command: string,
    options: { cwd: string; env: Record<string, string>; timeout?: number },
  ): { child: ChildProcess; timeoutHandle: NodeJS.Timeout } {
    const { cwd, env, timeout = 300000 } = options

    // Create shell command with proper environment setup
    // First check if .nvmrc exists to determine Node version
    const shellCommand =
      process.platform === 'win32'
        ? command
        : `export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"; [ -f .nvmrc ] && nvm use; ${command}`

    const shellExecutable =
      process.platform === 'win32' ? 'cmd.exe' : process.env.SHELL || '/bin/zsh'
    const shellArgs = process.platform === 'win32' ? ['/c', shellCommand] : ['-c', shellCommand]

    // Spawn with immediate stdio access
    const child = spawn(shellExecutable, shellArgs, {
      cwd,
      env: { ...process.env, ...env },
      stdio: ['ignore', 'pipe', 'pipe'],
    })

    // Set up timeout
    const timeoutHandle = setTimeout(() => {
      child.kill('SIGTERM')
    }, timeout)

    return { child, timeoutHandle }
  }

  async stop(timeout = 10): Promise<boolean> {
    // Check for orphaned process if no active process
    if (!this.process || this.process.exitCode !== null) {
      this.logger.debug('No server process found')
      return false
    }

    this.logger.info(`Stopping server with PID ${this.process.pid}`)

    try {
      // Mark that we're intentionally stopping to avoid error logs
      this.isIntentionallyStopping = true

      // Stop proxy server first
      await this.stopProxy()

      // Try graceful shutdown first - only kill the child process, not the process group!
      this.process.kill('SIGTERM')

      // Wait for graceful shutdown
      const shutdownPromise = new Promise<void>((resolve) => {
        this.process!.on('exit', () => resolve())
      })

      const timeoutPromise = new Promise<void>((_, reject) => {
        setTimeout(() => reject(new Error('Timeout')), timeout * 1000)
      })

      try {
        await Promise.race([shutdownPromise, timeoutPromise])
        this.logger.info('Server stopped gracefully')
      } catch {
        // Force kill if graceful shutdown failed - only kill the child process!
        this.logger.warn('Graceful shutdown timed out, force killing...')
        this.process.kill('SIGKILL')
        await shutdownPromise
        this.logger.info('Server force killed')
      }

      this.cleanup()
      await this.emitServerEvent('stopped')
      return true
    } catch (error) {
      this.logger.error('Error stopping server:', error as Error)

      this.cleanup()
      await this.emitServerEvent('stopped')
      return false
    }
  }

  async restart(
    serverCommands?: string | string[],
    envVars?: Record<string, string>,
    repositoryConfig?: RepositoryConfig,
    logFile?: string,
  ): Promise<ServerStartResult> {
    this.logger.info('Restarting server')

    // Stop the server first if it's running
    if (this.process && this.process.exitCode === null) {
      const stopped = await this.stop()
      if (!stopped) {
        this.logger.warn('Failed to stop server gracefully, proceeding with restart')
      }
    }

    // Use provided configuration or fall back to cached values
    const useServerCommands = serverCommands || this.serverCommands
    const useEnvVars = envVars || this.envVars
    const useRepositoryConfig = repositoryConfig || this.repositoryConfig

    if (!useServerCommands) {
      throw new Error(
        'Cannot restart server without configuration. Provide configuration or call start() first.',
      )
    }

    // Start the server again with configuration
    return await this.start(useServerCommands, useEnvVars, useRepositoryConfig, logFile)
  }

  /**
   * Get current server status based on process and port state.
   *
   * Status logic:
   * - No process → 'stopped'
   * - Process but no port → 'starting' (starting up, waiting for port detection)
   * - Process and port → 'started'
   * - Process exited → 'stopped' or 'failed' (based on failureReason)
   */
  getStatus(): ServerStatus {
    if (!this.process) {
      this.logger.debug('No process object')
      return {
        status: this.failureReason ? 'failed' : 'stopped',
        error: this.failureReason,
      }
    }

    if (this.process.exitCode !== null) {
      // Determine if this was a failure based on exit code
      // Any non-zero exit code indicates failure
      const isFailure = this.process.exitCode !== 0
      const errorMessage =
        this.failureReason ||
        (isFailure ? `Process exited with code ${this.process.exitCode}` : undefined)

      return {
        status: isFailure ? 'failed' : 'stopped',
        error: errorMessage,
      }
    }

    // Process is running - but we need to check if it has a valid port
    const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0

    // Only consider server "running" if we have a detected port
    if (this.port) {
      this.logger.debug(`Process ${this.process.pid} is running on port ${this.port}`)
      return {
        status: 'started',
        url: `http://localhost:${this.port}`,
        proxyUrl: this.proxyUrl,
        pid: this.process.pid,
        uptimeSeconds: uptime,
      }
    }
    // Process exists but no port detected - server is still starting up
    this.logger.debug(`Process ${this.process.pid} is starting - no port detected yet`)
    return {
      status: 'starting',
      pid: this.process.pid,
      uptimeSeconds: uptime,
    }
  }

  async getLogs(lines = 500): Promise<string> {
    if (!this.logFile || !(await pathExists(this.logFile))) {
      return 'No log file available'
    }

    try {
      if (process.platform !== 'win32') {
        // Use tail on Unix
        const { stdout } = await execAsync(`tail -n ${lines} "${this.logFile}"`)
        return stdout
      }
      // On Windows, read the whole file
      const content = await fs.readFile(this.logFile, 'utf-8')
      const allLines = content.split('\n')
      return allLines.slice(-lines).join('\n')
    } catch (error) {
      this.logger.error('Error reading logs:', error as Error)
      return `Error reading logs: ${error}`
    }
  }

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

    // Send existing content first if file exists
    if (await pathExists(this.logFile)) {
      try {
        const existingContent = await fs.readFile(this.logFile, 'utf-8')
        if (existingContent.trim()) {
          yield {
            type: 'chunk',
            data: {
              text: existingContent,
              timestamp: new Date().toISOString(),
            },
          }
        }
      } catch (error) {
        this.logger.error('Error reading existing logs:', error as Error)
      }
    }

    // If server is not running, just send what we have and complete
    if (!this.process || this.process.exitCode !== null) {
      yield {
        type: 'complete',
        data: {
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    // Watch for new log content
    try {
      let lastSize = 0
      if (await pathExists(this.logFile)) {
        const stat = await fs.stat(this.logFile)
        lastSize = stat.size
      }

      // Poll for file changes (in production, consider using fs.watch)
      while (this.process && this.process.exitCode === null) {
        await new Promise((resolve) => setTimeout(resolve, 500)) // Check every 500ms

        if (await pathExists(this.logFile)) {
          try {
            const stat = await fs.stat(this.logFile)

            if (stat.size > lastSize) {
              // File has grown, read the new content
              const newContent = await readFileRange(this.logFile, lastSize, stat.size - lastSize)
              if (newContent) {
                yield {
                  type: 'chunk',
                  data: {
                    text: newContent,
                    timestamp: new Date().toISOString(),
                  },
                }
              }
              lastSize = stat.size
            }
          } catch (error) {
            // File might be temporarily unavailable, continue watching
            this.logger.debug('Temporary error reading log file:', error)
          }
        }
      }

      // Send final completion event
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

  private cleanup(): void {
    if (this.process && this.process.exitCode == null) {
      throw new Error('Unexpected process state')
    }

    this.port = undefined
    this.startTime = undefined
    this.failureReason = undefined
    this.currentStatus = 'stopped'
    this.proxyPort = undefined
    this.proxyUrl = undefined
  }

  getPort(): number | undefined {
    return this.port
  }

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

      this.previewProxy = createPreviewProxy(
        targetUrl,
        scriptContent,
        this.logger as Logger,
        this.proxyPort,
      )
      await this.previewProxy.start()

      this.proxyUrl = `http://localhost:${this.proxyPort}`
      this.logger.info('Preview proxy started successfully', {
        proxyPort: this.proxyPort,
        proxyUrl: this.proxyUrl,
      })
    } catch (error) {
      this.logger.warn('Failed to start preview proxy, falling back to direct URL', {
        error: error instanceof Error ? error.message : String(error),
        proxyPort: this.proxyPort,
      })
      // Reset proxy-related variables since proxy failed
      this.previewProxy = undefined
      this.proxyPort = undefined
      this.proxyUrl = undefined
    }
  }

  /**
   * Stop the preview proxy server
   */
  private async stopProxy(): Promise<void> {
    if (this.previewProxy) {
      try {
        await this.previewProxy.stop()
        this.previewProxy = undefined
        this.proxyPort = undefined
        this.proxyUrl = undefined
        this.logger.info('Preview proxy stopped successfully')
      } catch (error) {
        this.logger.error('Error stopping preview proxy', error as Error)
      }
    }
  }

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
    // Use the detected port
    const targetPort = this.port

    if (!targetPort) {
      throw new Error('No port available for proxying - server may not be running')
    }

    // Check if we have a process or if the port is in use (server might be running independently)
    if (!this.portService.isPortInUseSync(targetPort)) {
      throw new Error(`Server is not running on port ${targetPort}`)
    }

    const url = `http://localhost:${targetPort}${request.path}`
    this.logger.debug('Proxying request', {
      url,
      method: request.method,
      targetPort,
      detectedPort: this.port,
      processRunning: this.process && this.process.exitCode === null,
    })

    try {
      const fetchOptions: RequestInit = {
        method: request.method,
        headers: request.headers,
      }

      if (request.body && request.method !== 'GET' && request.method !== 'HEAD') {
        fetchOptions.body = request.body
      }

      const response = await fetch(url, fetchOptions)
      const responseHeaders = new Headers()

      // Copy response headers, filtering out hop-by-hop headers
      const hopByHopHeaders = [
        'connection',
        'keep-alive',
        'proxy-authenticate',
        'proxy-authorization',
        'te',
        'trailer',
        'transfer-encoding',
        'upgrade',
      ]

      response.headers.forEach((value, key) => {
        if (!hopByHopHeaders.includes(key.toLowerCase())) {
          responseHeaders.set(key, value)
        }
      })

      // Read response body
      const contentType = response.headers.get('content-type') || ''
      let body: string | Buffer

      if (
        contentType.includes('text/') ||
        contentType.includes('application/json') ||
        contentType.includes('application/javascript')
      ) {
        body = await response.text()
      } else {
        body = Buffer.from(await response.arrayBuffer())
      }

      return {
        status: response.status,
        headers: responseHeaders,
        body,
      }
    } catch (error) {
      this.logger.error('Error proxying request', error as Error)
      throw error
    }
  }
}
