import type { ChildProcess } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import type {
  PreviewEnvironmentMetadata,
  RepositoryConfig,
  ServerStatus,
  ServerStatusEnum,
} from '@moatless/types'
import type { Logger } from 'pino'
import { monitorCommandOutput, type SpawnResult, spawnCommand } from '../utils/command-executor.js'
import { getDataDir } from '../utils/data-dir.js'
import { execAsync } from '../utils/git-utils.js'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
import { findHttpPortForPid } from '../utils/port-detection.js'
import { getPreviewInspectorScript } from '../utils/preview-inspector-script.js'
import type { EventService } from './event.service.js'
import { PortService } from './port.service.js'
import { createPreviewProxy, type PreviewProxyService } from './preview-proxy.service.js'

interface ServerStartResult {
  running: boolean
  port: number
  pid: number
  log_file: string
  uptime_seconds: number
  starting?: boolean
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
  private readonly portPlaceholder = '{PORT}'
  private currentStatus: ServerStatusEnum = 'stopped'
  private previewProxy?: PreviewProxyService
  private readonly previewEnvironmentsPath: string
  private proxyPort?: number
  private proxyUrl?: string
  private isIntentionallyStopping = false
  private initializeStoragePromise?: Promise<void>

  constructor(
    private taskId: string,
    private workspacePath: string,
    private serverCommands: string | string[],
    private envVars: Record<string, string> = {},
    private eventService?: EventService,
    private repositoryConfig?: RepositoryConfig,
  ) {
    // Normalize to array format
    const commandsArray = Array.isArray(serverCommands) ? serverCommands : [serverCommands]

    // Initialize preview environments storage path
    this.previewEnvironmentsPath = path.join(getDataDir(), 'preview-environments')
    // Defer storage initialization to avoid async in constructor
    this.initializeStoragePromise = this.initializeStorage()

    this.logger.info('PreviewServer initialized with:', {
      taskId: this.taskId,
      workspacePath,
      serverCommands: commandsArray,
      portPlaceholder: this.portPlaceholder,
      envVars: Object.keys(envVars),
      hasEventService: !!this.eventService,
      hasRepositoryConfig: !!this.repositoryConfig,
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

  async start(logFile?: string): Promise<ServerStartResult> {
    // Ensure storage is initialized before starting
    if (this.initializeStoragePromise) {
      await this.initializeStoragePromise
      this.initializeStoragePromise = undefined
    }

    if (this.process && this.process.exitCode === null) {
      throw new Error('Server is already running')
    }

    // Check if we already have a process starting up
    if (this.process && this.process.exitCode === null && !this.port) {
      // Return starting state if process exists but no port detected yet
      return {
        running: false,
        starting: true,
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
    await this.emitServerEvent('initializing')

    // 1. SYNCHRONOUSLY prepare command and spawn process to get PID immediately
    try {
      const { command, commandOptions } = await this.prepareServerCommand(commandsArray)

      // Create write stream for logs
      const logStream = await fs.open(this.logFile!, 'a')

      // SYNCHRONOUSLY spawn the process to get PID immediately
      this.logger.info('Spawning server process synchronously for immediate PID', { command })
      const spawnResult = spawnCommand(command, commandOptions)

      // Store process immediately - this fixes "No process object" issue
      this.process = spawnResult.child

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
          running: false,
          starting: false,
          port: 0,
          pid: 0,
          log_file: this.logFile,
          uptime_seconds: 0,
          error: this.failureReason,
        }
      }

      this.logger.info('Process spawned successfully - PID now available in start() return', {
        pid: this.process.pid,
        command: command,
      })

      // 2. Check for immediate process exit (catches failures like "exit 2")
      // First check synchronously in case it already exited
      if (this.process.exitCode !== null) {
        const exitCode = this.process.exitCode
        this.logger.info(`Process exited immediately with code ${exitCode}`)

        if (exitCode !== 0) {
          this.failureReason = `Process exited immediately with code ${exitCode}`
          await this.emitServerEvent('failed', this.failureReason)
        }

        // Close log stream before returning
        await logStream.close()

        // Return failure status
        return {
          running: false,
          starting: false,
          port: 0,
          pid: this.process?.pid || 0,
          log_file: this.logFile,
          uptime_seconds: 0,
          error: this.failureReason,
        }
      }

      // Create promise for early completion detection
      let earlyCompleteResolve: (() => void) | undefined
      const earlyCompletePromise = new Promise<void>((resolve) => {
        earlyCompleteResolve = resolve
      })

      // 3. Start ASYNC monitoring in background with early completion callback
      this.monitorServerProcess(spawnResult, command, logStream, earlyCompleteResolve).catch(
        (error) => {
          this.logger.error('Background monitoring process failed', error)
          this.failureReason = error instanceof Error ? error.message : String(error)
          this.emitServerEvent('failed', this.failureReason).catch((eventError) => {
            this.logger.error('Failed to emit server failed event', eventError)
          })
          // Don't call cleanup() here - the process might still be running
          // Only set the failure reason so getStatus() can report the error
        },
      )

      // Wait briefly for early completion or timeout
      const earlyResult = await Promise.race([
        earlyCompletePromise.then(() => 'completed' as const),
        new Promise<'timeout'>((resolve) => setTimeout(() => resolve('timeout'), 150)),
      ])

      // Check if process failed early (monitor detected failure)
      if (earlyResult === 'completed' && this.failureReason) {
        this.logger.info('Early failure detected by monitor', { reason: this.failureReason })
        await this.emitServerEvent('failed', this.failureReason)

        return {
          running: false,
          starting: false,
          port: 0,
          pid: this.process?.pid || 0,
          log_file: this.logFile,
          uptime_seconds: 0,
          error: this.failureReason, // Includes stderr from monitor!
        }
      }

      // 4. Save metadata for recovery
      const metadata: PreviewEnvironmentMetadata = {
        taskId: this.taskId,
        pid: this.process?.pid || 0,
        port: this.port || 0,
        startTime: new Date(),
        command: Array.isArray(this.serverCommands)
          ? this.serverCommands.join(' && ')
          : this.serverCommands,
        workspacePath: this.workspacePath,
        logFile: this.logFile || '',
        processStartTime: Date.now(),
        envVars: this.envVars,
        failed: false,
        error: undefined,
        logs: undefined,
        starting: true,
        proxyPort: this.proxyPort,
        proxyUrl: this.proxyUrl,
      }
      await this.savePreviewEnvironmentMetadata(metadata)

      // 5. Return immediately with PID available
      return {
        running: false,
        starting: true,
        port: this.port || 0,
        pid: this.process?.pid || 0, // Now has actual PID!
        log_file: this.logFile,
        uptime_seconds: 0,
      }
    } catch (error) {
      this.logger.error('Failed to spawn server process', error as Error)
      this.failureReason = error instanceof Error ? error.message : String(error)
      await this.emitServerEvent('failed', this.failureReason)
      // Don't call cleanup() here - only cleanup when starting again or stopping
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
    const originalCommand = combinedCommand
    let command = combinedCommand

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
      this.logger.info('No port placeholder found - will detect actual port from server startup:', {
        originalCommand,
        placeholder: this.portPlaceholder,
      })
    }

    this.logger.info(`Final server command: ${command}`)
    this.logger.info(`Server logs will be saved to: ${this.logFile}`)

    const commandOptions = {
      cwd: this.workspacePath,
      env: this.envVars,
      timeout: 300000, // 5 minute timeout for server startup
    }

    return { command, commandOptions }
  }

  private async monitorServerProcess(
    spawnResult: SpawnResult,
    command: string,
    logStream: fs.FileHandle,
    onEarlyComplete?: () => void,
  ): Promise<void> {
    let portDetected = false
    let lastPortCheck = 0
    const PORT_CHECK_INTERVAL = 1000 // Check every second for PID-based port detection
    this.startTime = Date.now()

    try {
      this.logger.info('Starting async monitoring of spawned server process', {
        pid: this.process?.pid,
        command,
      })

      // Monitor the already spawned process for output and port detection
      for await (const event of monitorCommandOutput(spawnResult, command)) {
        switch (event.type) {
          case 'start':
            // Start event - process was already spawned, so this is just confirmation
            this.logger.debug('Monitor started for spawned process', { pid: event.data.pid })
            break

          case 'output':
            // Write output to log file
            try {
              if (event.data.text) {
                await logStream.write(event.data.text)

                // Use PID-based port detection instead of output parsing
                if (!portDetected && this.process?.pid) {
                  const now = Date.now()

                  // Check for port every PORT_CHECK_INTERVAL milliseconds
                  if (now - lastPortCheck > PORT_CHECK_INTERVAL) {
                    lastPortCheck = now

                    // Try to find HTTP port for this process and its children
                    const port = await findHttpPortForPid(this.process.pid)
                    if (port) {
                      this.port = port
                      portDetected = true

                      // Start proxy server after port is detected
                      await this.startProxy()

                      // Update metadata with running status and proxy info
                      const metadata: PreviewEnvironmentMetadata = {
                        taskId: this.taskId,
                        pid: this.process?.pid || 0,
                        port: this.port,
                        startTime: new Date(this.startTime || Date.now()),
                        command: Array.isArray(this.serverCommands)
                          ? this.serverCommands.join(' && ')
                          : this.serverCommands,
                        workspacePath: this.workspacePath,
                        logFile: this.logFile || '',
                        processStartTime: this.startTime || Date.now(),
                        envVars: this.envVars,
                        failed: false,
                        error: undefined,
                        logs: undefined,
                        starting: false,
                        proxyPort: this.proxyPort,
                        proxyUrl: this.proxyUrl,
                      }
                      await this.savePreviewEnvironmentMetadata(metadata)

                      await this.emitServerEvent('running')
                      this.logger.info(
                        `Detected server running on port ${port} via PID ${this.process.pid}`,
                      )
                    } else {
                      this.logger.debug(`No HTTP port found yet for PID ${this.process.pid}`)
                    }
                  }
                }
              }
            } catch (writeError) {
              this.logger.warn('Failed to write to log file', writeError)
            }
            break

          case 'complete':
            // Signal early completion if within first 200ms
            if (onEarlyComplete && Date.now() - this.startTime < 200) {
              onEarlyComplete()
            }

            // Close log file
            try {
              await logStream.close()
            } catch (closeError) {
              this.logger.warn('Failed to close log file', closeError)
            }

            // If process exits without port detected, it's a failure (not a server)
            if (!portDetected && !this.isIntentionallyStopping) {
              const errorMsg = `Process exited without starting a server (exit code: ${event.data.exitCode})`
              this.logger.error('Server failed - no port detected', {
                exitCode: event.data.exitCode,
              })
              this.failureReason = errorMsg
              await this.emitServerEvent('failed', this.failureReason)
              return
            }

            // Handle normal exit codes
            if (event.data.exitCode === 0 || this.isIntentionallyStopping) {
              this.logger.info('Server command completed successfully')
            } else {
              // Non-zero exit with port detected - still a failure
              const errorMsg = event.data.error || `Command exited with code ${event.data.exitCode}`
              this.logger.error('Server command failed', { error: errorMsg })
              this.failureReason = errorMsg
              await this.emitServerEvent('failed', this.failureReason)
              return
            }
            break
        }
      }
    } catch (error) {
      // Close log file on error
      try {
        await logStream.close()
      } catch (closeError) {
        this.logger.warn('Failed to close log file on error', closeError)
      }

      const errorMsg = error instanceof Error ? error.message : String(error)
      this.logger.error('Command execution failed', { error: errorMsg })
      this.failureReason = errorMsg
      await this.emitServerEvent('failed', this.failureReason)
      // Don't cleanup here - let the process continue if it's still running
      // The error might be in monitoring, not the actual process
      throw error
    }
  }

  async stop(timeout = 10): Promise<boolean> {
    // Check for orphaned process if no active process
    if (!this.process || this.process.exitCode !== null) {
      this.logger.debug('No server process found')

      // Check if we have metadata for a potentially orphaned process
      const metadata = await this.loadPreviewEnvironmentMetadata()
      if (metadata) {
        this.logger.info('Found orphaned server metadata, cleaning up metadata only', {
          pid: metadata.pid,
        })
        // Don't kill the process - it might be something important!
        // Just clean up the metadata
        await this.deletePreviewEnvironmentMetadata()
        return true
      }
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

      // Clean up metadata
      await this.deletePreviewEnvironmentMetadata()

      this.cleanup()
      await this.emitServerEvent('stopped')
      return true
    } catch (error) {
      this.logger.error('Error stopping server:', error as Error)

      // Force cleanup
      await this.deletePreviewEnvironmentMetadata()
      this.cleanup()
      await this.emitServerEvent('stopped')
      return false
    }
  }

  async restart(logFile?: string): Promise<ServerStartResult> {
    this.logger.info('Restarting server')

    // Stop the server first if it's running
    if (this.process && this.process.exitCode === null) {
      const stopped = await this.stop()
      if (!stopped) {
        this.logger.warn('Failed to stop server gracefully, proceeding with restart')
      }
    }

    // Start the server again
    return await this.start(logFile)
  }

  /**
   * Get current server status based on process and port state.
   *
   * Status logic:
   * - No process → 'stopped'
   * - Process but no port → 'initializing' (starting up, waiting for port detection)
   * - Process and port → 'running'
   * - Process exited → 'stopped' or 'failed' (based on failureReason)
   */
  getStatus(configured = true): ServerStatus {
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
          proxyUrl: this.proxyUrl,
          pid: undefined,
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
      // Process has exited - check if port is still in use
      if (this.port && this.portService.isPortInUseSync(this.port)) {
        this.logger.debug(`Process exited but port ${this.port} is still in use`)
        const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
        return {
          running: true,
          configured,
          url: `http://localhost:${this.port}`,
          pid: this.process?.pid,
          uptimeSeconds: uptime,
        }
      }

      // Determine if this was a failure based on exit code
      // Any non-zero exit code indicates failure
      const isFailure = this.process.exitCode !== 0
      const errorMessage =
        this.failureReason ||
        (isFailure ? `Process exited with code ${this.process.exitCode}` : undefined)

      // Don't cleanup here - preserve log access after failure
      return {
        running: false,
        configured,
        status: isFailure ? 'failed' : 'stopped',
        uptimeSeconds: 0,
        error: errorMessage,
      }
    }

    // Process is running - but we need to check if it has a valid port
    const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0

    // Only consider server "running" if we have a detected port
    if (this.port) {
      this.logger.debug(`Process ${this.process.pid} is running on port ${this.port}`)
      return {
        running: true,
        configured,
        status: 'running',
        url: `http://localhost:${this.port}`,
        proxyUrl: this.proxyUrl,
        pid: this.process.pid,
        uptimeSeconds: uptime,
      }
    }
    // Process exists but no port detected - server is still starting up
    this.logger.debug(`Process ${this.process.pid} is starting - no port detected yet`)
    return {
      running: false,
      configured,
      starting: true,
      status: 'initializing',
      pid: this.process.pid,
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
      if (await this.pathExists(this.logFile)) {
        const stat = await fs.stat(this.logFile)
        lastSize = stat.size
      }

      // Poll for file changes (in production, consider using fs.watch)
      while (this.process && this.process.exitCode === null) {
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

    // Only clear the process reference if it has actually exited
    // This prevents losing track of running processes when monitoring fails
    if (this.process && this.process.exitCode !== null) {
      this.process = undefined
    }

    this.port = undefined
    this.startTime = undefined
    // Don't clear failureReason - we need to preserve it for getStatus()
    // this.failureReason = undefined
    this.currentStatus = 'stopped'
    this.proxyPort = undefined
    this.proxyUrl = undefined
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

  // ===============================
  // Metadata Persistence Methods
  // ===============================

  /**
   * Save preview environment metadata to disk
   */
  private async savePreviewEnvironmentMetadata(
    metadata: PreviewEnvironmentMetadata,
  ): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${this.taskId}.json`)
    await fs.writeFile(filePath, JSON.stringify(metadata, null, 2), 'utf-8')
  }

  /**
   * Load preview environment metadata from disk
   */
  private async loadPreviewEnvironmentMetadata(): Promise<PreviewEnvironmentMetadata | null> {
    const filePath = path.join(this.previewEnvironmentsPath, `${this.taskId}.json`)
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
  private async deletePreviewEnvironmentMetadata(): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${this.taskId}.json`)
    try {
      await fs.unlink(filePath)
    } catch {
      // Ignore errors if file doesn't exist
    }
  }

  // ===============================
  // Process Management Methods
  // ===============================

  /**
   * Initialize storage directory
   */
  private async initializeStorage(): Promise<void> {
    try {
      await fs.mkdir(this.previewEnvironmentsPath, { recursive: true })
    } catch (error) {
      this.logger.error('Failed to initialize preview environment storage', error as Error)
    }
  }
}
