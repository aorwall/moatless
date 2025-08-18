import { type ChildProcess, spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import type { ServerStatus, ServerStatusEnum } from '@moatless/types'
import { executeCommandWithStream } from '../utils/command-executor.js'
import { execAsync } from '../utils/git-utils.js'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
import type { EventService } from './event.service.js'
import { PortService } from './port.service.js'

interface ServerStartResult {
  running: boolean
  port: number
  pid: number
  log_file: string
  uptime_seconds: number
  starting?: boolean
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
  private actualPort?: number
  private pid?: number
  private logFile?: string
  private startTime?: number
  private failureReason?: string
  private logger = createTaskLogger('preview-server')
  private portService = new PortService(this.logger)
  private readonly portPlaceholder = '{PORT}'
  private isStarting = false
  private currentStatus: ServerStatusEnum = 'stopped'

  constructor(
    private taskId: string,
    private workspacePath: string,
    private serverCommands: string | string[],
    private envVars: Record<string, string> = {},
    private eventService?: EventService,
  ) {
    // Normalize to array format
    const commandsArray = Array.isArray(serverCommands) ? serverCommands : [serverCommands]

    this.logger.info('PreviewServer initialized with:', {
      taskId: this.taskId,
      workspacePath,
      serverCommands: commandsArray,
      portPlaceholder: this.portPlaceholder,
      envVars: Object.keys(envVars),
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
        subType:
          status === 'initializing'
            ? 'starting'
            : status === 'running'
              ? 'started'
              : status === 'stopped'
                ? 'stopped'
                : 'error',
        taskId: this.taskId,
        timestamp: new Date(),
        isError: status === 'failed',
        payload: {
          status,
          previousStatus,
          port: this.actualPort || this.port,
          pid: this.pid,
          url:
            this.actualPort || this.port
              ? `http://localhost:${this.actualPort || this.port}`
              : undefined,
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

  async start(logFile?: string): Promise<ServerStartResult> {
    if (this.process && this.process.exitCode === null) {
      throw new Error('Server is already running')
    }

    if (this.isStarting) {
      // Return starting state if startup is in progress
      return {
        running: false,
        starting: true,
        port: this.port || 0,
        pid: this.pid || 0,
        log_file: this.logFile || '',
        uptime_seconds: 0,
      }
    }

    // Clean up any previous state
    this.cleanup()

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

    // Set starting state immediately
    this.isStarting = true

    // Emit initializing event
    await this.emitServerEvent('initializing')

    // Start background startup process (no need to await)
    this.runStartupInBackground(commandsArray).catch((error) => {
      this.logger.error('Background startup process failed', error)
    })

    // Return immediately with starting status
    return {
      running: false,
      starting: true,
      port: this.port || 0,
      pid: this.pid || 0,
      log_file: this.logFile,
      uptime_seconds: 0,
    }
  }

  private async runStartupInBackground(commandsArray: string[]): Promise<void> {
    try {
      this.logger.info('Starting background server startup process')

      // Execute preparation commands (all but the last one)
      const preparationCommands = commandsArray.slice(0, -1)
      const serverCommand = commandsArray[commandsArray.length - 1]

      if (preparationCommands.length > 0) {
        this.logger.info(
          `Executing ${preparationCommands.length} preparation commands before starting server`,
        )

        for (let i = 0; i < preparationCommands.length; i++) {
          const prepCommand = preparationCommands[i]
          this.logger.info(`Executing preparation command ${i + 1}: ${prepCommand}`)

          try {
            await this.executeCommand(prepCommand)
            this.logger.info(`Preparation command ${i + 1} completed successfully`)
          } catch (error) {
            const errorMsg = `Preparation command ${i + 1} failed: ${error instanceof Error ? error.message : String(error)}`
            this.logger.error(errorMsg)
            this.failureReason = errorMsg
            this.isStarting = false
            await this.emitServerEvent('failed', errorMsg)
            return
          }
        }
      }

      // Now start the actual server with the last command
      const originalCommand = serverCommand
      let command = serverCommand

      // Only allocate a port if the command contains the placeholder
      if (originalCommand.includes(this.portPlaceholder)) {
        const allocatedPort = await findAvailablePort('preview')
        this.port = allocatedPort
        command = originalCommand.replace(this.portPlaceholder, String(this.port))

        this.logger.info('Port placeholder replacement:', {
          originalCommand,
          placeholder: this.portPlaceholder,
          allocatedPort: this.port,
          finalCommand: command,
        })
      } else {
        this.logger.info(
          'No port placeholder found - will detect actual port from server startup:',
          {
            originalCommand,
            placeholder: this.portPlaceholder,
          },
        )
      }

      this.logger.info(`Starting server with command: ${command}`)
      this.logger.info(`Server logs will be saved to: ${this.logFile}`)

      // Create write stream for logs
      const logStream = await fs.open(this.logFile!, 'a')

      try {
        // Merge environment variables with current process environment
        const processEnv = {
          ...process.env,
          ...this.envVars,
        }

        // Start the process
        this.process = spawn(command, [], {
          cwd: this.workspacePath,
          shell: true,
          detached: process.platform !== 'win32',
          stdio: ['ignore', logStream.fd, logStream.fd],
          env: processEnv,
        })

        this.pid = this.process.pid
        this.startTime = Date.now()

        // Start background port detection
        this.startPortDetection()
      } finally {
        await logStream.close()
      }

      this.isStarting = false
      this.logger.info('Background server startup initiated')
    } catch (error) {
      this.logger.error('Background startup failed', error as Error)
      this.failureReason = error instanceof Error ? error.message : String(error)
      this.isStarting = false
      await this.emitServerEvent('failed', this.failureReason)
      this.cleanup()
    }
  }

  private async startPortDetection(): Promise<void> {
    // Port detection runs in background without blocking
    const maxWait = 10 // seconds
    const checkInterval = 0.5
    let elapsed = 0

    this.logger.debug(`Starting background port detection (up to ${maxWait} seconds)`)

    while (elapsed < maxWait && this.process && this.process.exitCode === null) {
      await new Promise((resolve) => setTimeout(resolve, checkInterval * 1000))
      elapsed += checkInterval

      if (this.port) {
        // We have an allocated port, check if it's in use
        if (await this.portService.isPortInUse(this.port)) {
          this.logger.info(`Server is now listening on allocated port ${this.port}`)
          await this.emitServerEvent('running')
          return // Port detected successfully
        }
      }

      // Always try to detect actual port from logs with PID validation
      if (this.logFile && this.pid) {
        const actualPort = await this.portService.detectAndValidatePort(this.logFile, this.pid)
        if (actualPort) {
          this.logger.info(`Detected and validated server running on port ${actualPort} from logs`)
          this.actualPort = actualPort
          this.port = actualPort
          await this.emitServerEvent('running')
          return // Port detected successfully
        }
      }
    }

    // Final port detection attempt
    if (!this.port && this.logFile && this.pid) {
      const actualPort = await this.portService.detectAndValidatePort(this.logFile, this.pid)
      if (actualPort) {
        this.logger.info(`Final detection: validated server running on port ${actualPort}`)
        this.actualPort = actualPort
        this.port = actualPort
        await this.emitServerEvent('running')
      }
    }

    // Check final status
    if (this.process && this.process.exitCode !== null) {
      const portInUse = this.port ? await this.portService.isPortInUse(this.port) : false
      if (!portInUse || !this.port) {
        this.failureReason = `Server process exited with code ${this.process.exitCode}${this.port ? '' : ' (no port detected)'}`
        this.logger.error('Server startup failed:', this.failureReason)
      } else {
        this.logger.info(`Parent process exited but server is running on port ${this.port}`)
      }
    } else if (!this.port) {
      this.failureReason = 'Failed to detect server port after startup'
      this.logger.error(this.failureReason)
    } else {
      this.logger.info(`Background port detection completed - server running on port ${this.port}`)
    }
  }

  async stop(timeout = 10): Promise<boolean> {
    if (!this.process || this.process.exitCode !== null) {
      this.logger.warn('Server is not running')
      return false
    }

    this.logger.info(`Stopping server with PID ${this.pid}`)

    try {
      // Try graceful shutdown first
      if (process.platform !== 'win32') {
        // On Unix, send SIGTERM to process group
        process.kill(-this.pid!, 'SIGTERM')
      } else {
        // On Windows, just kill the process
        this.process.kill('SIGTERM')
      }

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
        // Force kill if graceful shutdown failed
        this.logger.warn('Graceful shutdown timed out, force killing...')
        if (process.platform !== 'win32') {
          process.kill(-this.pid!, 'SIGKILL')
        } else {
          this.process.kill('SIGKILL')
        }
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

  getStatus(configured = true): ServerStatus {
    // Check if we're in startup phase
    if (this.isStarting) {
      return {
        running: false,
        configured,
        starting: true,
        status: 'initializing',
        uptimeSeconds: 0,
      }
    }

    // Check if process is running
    if (!this.process) {
      this.logger.debug('No process object')
      // Even without process, check if port is in use
      if (this.port && this.portService.isPortInUseSync(this.port)) {
        this.logger.debug(`No process object but port ${this.port} is in use`)
        const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
        return {
          running: true,
          configured,
          status: 'running',
          url: `http://localhost:${this.port}`,
          pid: this.pid,
          uptimeSeconds: uptime,
        }
      }
      return {
        running: false,
        configured,
        status: this.failureReason ? 'failed' : 'stopped',
        uptimeSeconds: 0,
        error: this.failureReason,
      }
    }

    if (this.process.exitCode !== null) {
      this.logger.debug(`Process exited with code: ${this.process.exitCode}`)
      // Check if port is still in use
      if (this.port && this.portService.isPortInUseSync(this.port)) {
        this.logger.debug(`Process exited but port ${this.port} is still in use`)
        const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
        return {
          running: true,
          configured,
          url: `http://localhost:${this.port}`,
          pid: this.pid,
          uptimeSeconds: uptime,
        }
      }
      // Don't cleanup here - preserve log access after failure
      return {
        running: false,
        configured,
        status: this.failureReason ? 'failed' : 'stopped',
        uptimeSeconds: 0,
        error: this.failureReason,
      }
    }

    // Process is running
    const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
    this.logger.debug(`Process ${this.pid} is running on port ${this.port}`)
    return {
      running: true,
      configured,
      status: 'running',
      url: `http://localhost:${this.port}`,
      pid: this.pid,
      uptimeSeconds: uptime,
    }
  }

  async getLogs(lines = 100): Promise<string> {
    if (!this.logFile || !(await this.pathExists(this.logFile))) {
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
    if (await this.pathExists(this.logFile)) {
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

    // If server is not starting and not running, just send what we have and complete
    if (!this.isStarting && (!this.process || this.process.exitCode !== null)) {
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
      if (await this.pathExists(this.logFile)) {
        const stat = await fs.stat(this.logFile)
        lastSize = stat.size
      }

      // Poll for file changes (in production, consider using fs.watch)
      while (this.isStarting || (this.process && this.process.exitCode === null)) {
        await new Promise((resolve) => setTimeout(resolve, 500)) // Check every 500ms

        if (await this.pathExists(this.logFile)) {
          try {
            const stat = await fs.stat(this.logFile)

            if (stat.size > lastSize) {
              // File has grown, read the new content
              const newContent = await this.readFileRange(
                this.logFile,
                lastSize,
                stat.size - lastSize,
              )
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

  private async readFileRange(filePath: string, start: number, length: number): Promise<string> {
    const buffer = Buffer.alloc(length)
    const fileHandle = await fs.open(filePath, 'r')

    try {
      const { bytesRead } = await fileHandle.read(buffer, 0, length, start)
      return buffer.subarray(0, bytesRead).toString('utf-8')
    } finally {
      await fileHandle.close()
    }
  }

  // Port detection methods now handled by PortService

  private cleanup(): void {
    // Release port allocation
    if (this.port) {
      // Port tracking simplified - no need to release ports manually
    }

    this.process = undefined
    this.pid = undefined
    this.port = undefined
    this.actualPort = undefined
    this.startTime = undefined
    this.failureReason = undefined
    this.isStarting = false
    this.currentStatus = 'stopped'
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  getPort(): number | undefined {
    return this.actualPort || this.port
  }

  getActualPort(): number | undefined {
    return this.actualPort
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
    // Use actualPort if available, otherwise fall back to port
    const targetPort = this.actualPort || this.port

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
      actualPort: this.actualPort,
      allocatedPort: this.port,
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

  /**
   * Execute a single command and wait for it to complete
   * Logs all output to the log file for complete visibility
   */
  private async executeCommand(command: string): Promise<void> {
    const startTime = new Date().toISOString()

    // Write command header to log file
    if (this.logFile) {
      const header = `\n========================================\n[${startTime}] Executing: ${command}\n========================================\n`
      await fs.appendFile(this.logFile, header)
    }

    let commandSuccess = true
    let commandError = ''

    try {
      // Use the shared utility to execute command with streaming
      for await (const event of executeCommandWithStream(command, {
        cwd: this.workspacePath,
        env: this.envVars,
        timeout: 300000, // 5 minutes timeout for preparation commands
      })) {
        // Log different event types
        switch (event.type) {
          case 'start':
            this.logger.info(`Starting preparation command: ${command}`)
            break

          case 'output':
            // Append output to log file in real-time
            if (this.logFile && event.data.text) {
              const prefix = event.data.outputType === 'stderr' ? '[stderr] ' : '[stdout] '
              await fs.appendFile(this.logFile, prefix + event.data.text)
            }

            // Also log to internal logger for debugging
            if (event.data.text) {
              if (event.data.outputType === 'stderr') {
                this.logger.debug(`Command stderr: ${event.data.text.trim()}`)
              } else {
                this.logger.debug(`Command stdout: ${event.data.text.trim()}`)
              }
            }
            break

          case 'complete': {
            const endTime = new Date().toISOString()
            const exitCode = event.data.exitCode ?? 1
            const success = exitCode === 0

            // Write completion footer to log file
            if (this.logFile) {
              const footer = `\n========================================\n[${endTime}] Command completed: ${command} (exit code: ${exitCode})\n========================================\n`
              await fs.appendFile(this.logFile, footer)
            }

            if (success) {
              this.logger.info(`Preparation command completed successfully: ${command}`)
            } else {
              commandSuccess = false
              commandError = event.data.error || `Command failed with exit code ${exitCode}`
              this.logger.error(`Preparation command failed: ${command}`, {
                exitCode,
                error: event.data.error,
              })
            }
            break
          }

          case 'error':
            commandSuccess = false
            commandError = 'Command execution error'
            this.logger.error(`Command execution error: ${command}`)
            break
        }
      }

      if (!commandSuccess) {
        throw new Error(commandError)
      }
    } catch (error) {
      // Write error to log file
      if (this.logFile) {
        const errorTime = new Date().toISOString()
        const errorMsg = `\n[${errorTime}] ERROR: ${error instanceof Error ? error.message : String(error)}\n`
        await fs.appendFile(this.logFile, errorMsg)
      }

      throw error
    }
  }
}
