import { type ChildProcess, spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  ShellCommandInfo,
  ShellCommandLogEvent,
  ShellCommandOptions,
  ShellCommandResult,
  ShellCommandStatus,
} from '@moatless/types'
import { getSafeSystemEnvironment } from '../utils/environment-utils.js'
import { logger } from '../utils/logger.js'
import { treeKill } from '../utils/tree-kill.js'

/**
 * Represents and manages a single shell command execution.
 * Each instance handles one command at a time.
 */
export class ShellCommand {
  private process: ChildProcess | undefined
  private status: ShellCommandStatus = 'idle'
  private command: string | undefined
  private pid: number | undefined
  private startTime: Date | undefined
  private endTime: Date | undefined
  private exitCode: number | undefined
  private workingDirectory: string | undefined
  private shell: string | undefined
  private error: string | undefined
  private logBuffer: ShellCommandLogEvent[] = []
  private logFile: string | undefined
  private timeoutHandle: NodeJS.Timeout | undefined
  private hasExecuted = false

  /**
   * Execute a shell command
   */
  async execute(command: string, options?: ShellCommandOptions): Promise<void> {
    if (this.hasExecuted) {
      throw new Error('ShellCommand can only be executed once')
    }

    if (this.status === 'running') {
      throw new Error('Command is already running')
    }

    // Mark as executed
    this.hasExecuted = true

    const workingDirectory = options?.workingDirectory || process.cwd()
    const shell = options?.shell || (process.platform === 'win32' ? 'cmd.exe' : '/bin/bash')
    const timeout = options?.timeout
    const background = options?.background ?? true

    logger.info('Executing shell command', {
      command,
      workingDirectory,
      shell,
      background,
      envVarCount: Object.keys(options?.env || {}).length,
      envVarKeys: Object.keys(options?.env || {}),
    })

    // Store command info
    this.command = command
    this.workingDirectory = workingDirectory
    this.shell = shell
    this.logFile = options?.logFile
    this.status = 'running'
    this.startTime = new Date()

    // Create the shell command with proper environment setup
    const shellCommand =
      process.platform === 'win32'
        ? command
        : `export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"; ${command}`

    const shellArgs = process.platform === 'win32' ? ['/c', shellCommand] : ['-c', shellCommand]

    // Spawn the process
    // Use 'pipe' for stdin instead of 'ignore' to allow dev servers (like Next.js) to detect port conflicts
    // When stdin is ignored, some dev servers skip port availability checks
    // Build environment: safe system vars + workspace-specific vars (no backend server vars)
    const processEnv = {
      ...getSafeSystemEnvironment(),
      ...options?.env,
    }

    this.process = spawn(shell, shellArgs, {
      cwd: workingDirectory,
      env: processEnv,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    this.pid = this.process.pid

    // Close stdin pipe to signal EOF to child process
    // This allows dev servers to know they're running non-interactively
    if (this.process.stdin) {
      this.process.stdin.end()
    }

    // Set up timeout if specified
    if (timeout) {
      this.timeoutHandle = setTimeout(() => {
        this.kill().catch((error) => {
          logger.error('Failed to kill timed out command', error as Error)
        })
        this.status = 'killed'
        this.error = 'Command timed out'
      }, timeout)
    }

    // Set up log streaming
    this.setupLogStreaming()

    // Set up process exit handling
    this.process.on('exit', (code, signal) => {
      this.handleProcessExit(code, signal)
    })

    this.process.on('error', (error) => {
      this.handleProcessError(error)
    })

    // If not background, wait for completion
    if (!background) {
      await new Promise<void>((resolve) => {
        this.process?.on('exit', () => resolve())
        this.process?.on('error', () => resolve())
      })
    }
  }

  private setupLogStreaming(): void {
    if (!this.process) {
      return
    }

    // Stream stdout
    if (this.process.stdout) {
      this.process.stdout.on('data', (chunk) => {
        const text = chunk.toString()
        const logEvent: ShellCommandLogEvent = {
          type: 'stdout',
          timestamp: new Date().toISOString(),
          text,
        }
        this.logBuffer.push(logEvent)
        this.persistLog(logEvent)
      })
    }

    // Stream stderr
    if (this.process.stderr) {
      this.process.stderr.on('data', (chunk) => {
        const text = chunk.toString()
        const logEvent: ShellCommandLogEvent = {
          type: 'stderr',
          timestamp: new Date().toISOString(),
          text,
        }
        this.logBuffer.push(logEvent)
        this.persistLog(logEvent)
      })
    }

    // Add system log for start
    const startEvent: ShellCommandLogEvent = {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Command started: ${this.command}\nPID: ${this.pid}\n`,
    }
    this.logBuffer.push(startEvent)
    this.persistLog(startEvent)
  }

  private async persistLog(logEvent: ShellCommandLogEvent): Promise<void> {
    if (!this.logFile) {
      return
    }

    try {
      // Ensure directory exists before writing
      const dir = path.dirname(this.logFile)
      await fs.mkdir(dir, { recursive: true })

      const logLine = `[${logEvent.timestamp}] [${logEvent.type}] ${logEvent.text}`
      await fs.appendFile(this.logFile, logLine)
    } catch (error) {
      logger.error('Failed to persist log', error as Error, {
        logFile: this.logFile,
      })
    }
  }

  private handleProcessExit(code: number | null, signal: string | null): void {
    // Clear timeout if exists
    if (this.timeoutHandle) {
      clearTimeout(this.timeoutHandle)
    }

    // Update command status
    this.endTime = new Date()
    this.exitCode = code || 0

    if (signal === 'SIGTERM' || signal === 'SIGKILL') {
      this.status = 'killed'
    } else if (code === 0) {
      this.status = 'completed'
    } else {
      this.status = 'failed'
    }

    // Add system log for completion
    const endEvent: ShellCommandLogEvent = {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Command ${this.status}: exit code ${code}, signal ${signal}\n`,
    }
    this.logBuffer.push(endEvent)
    this.persistLog(endEvent)

    logger.info('Shell command exited', {
      command: this.command,
      status: this.status,
      exitCode: code,
      signal,
      duration: this.startTime ? this.endTime.getTime() - this.startTime.getTime() : 0,
    })
  }

  private handleProcessError(error: Error): void {
    this.status = 'failed'
    this.error = error.message
    this.endTime = new Date()

    // Add error to logs
    const errorEvent: ShellCommandLogEvent = {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Error: ${error.message}\n`,
    }
    this.logBuffer.push(errorEvent)
    this.persistLog(errorEvent)

    logger.error('Shell command error', error, {
      command: this.command,
    })
  }

  /**
   * Kill the running command and all its child processes
   */
  async kill(): Promise<boolean> {
    if (!this.pid || this.status !== 'running') {
      return false
    }

    const pid = this.pid

    try {
      // Use tree-kill to recursively kill all child processes
      // This ensures child processes like esbuild are also terminated
      return await new Promise<boolean>((resolve) => {
        // Try graceful termination first with SIGTERM
        treeKill(pid, 'SIGTERM', (err) => {
          if (err) {
            logger.warn('SIGTERM failed, trying SIGKILL', { error: err })
            // If SIGTERM fails, force kill with SIGKILL
            treeKill(pid, 'SIGKILL', (killErr) => {
              if (killErr) {
                logger.error('Failed to kill command tree', killErr as Error)
                resolve(false)
              } else {
                resolve(true)
              }
            })
          } else {
            resolve(true)
          }
        })
      })
    } catch (error) {
      logger.error('Failed to kill command', error as Error)
      return false
    }
  }

  /**
   * Get the current status of the command
   */
  getStatus(): ShellCommandStatus {
    return this.status
  }

  /**
   * Get logs from the command
   */
  getLogs(lines?: number): ShellCommandLogEvent[] {
    if (lines && lines > 0) {
      const startIndex = Math.max(0, this.logBuffer.length - lines)
      return this.logBuffer.slice(startIndex)
    }
    return [...this.logBuffer]
  }

  /**
   * Stream logs from the command
   */
  async *streamLogs(): AsyncGenerator<ShellCommandLogEvent> {
    // First, yield existing logs
    for (const log of this.logBuffer) {
      yield log
    }

    // If command is still running, stream new logs
    if (this.status === 'running') {
      let lastIndex = this.logBuffer.length

      // Poll for new logs
      while (this.status === 'running') {
        const newLogs = this.logBuffer.slice(lastIndex)
        for (const log of newLogs) {
          yield log
        }
        lastIndex = this.logBuffer.length

        // Wait a bit before checking again
        await new Promise((resolve) => setTimeout(resolve, 100))
      }

      // Yield any final logs after command completes
      const finalLogs = this.logBuffer.slice(lastIndex)
      for (const log of finalLogs) {
        yield log
      }
    }
  }

  /**
   * Get information about the command
   */
  getInfo(): ShellCommandInfo | null {
    if (!this.command) {
      return null
    }

    return {
      command: this.command,
      status: this.status,
      pid: this.pid,
      startTime: this.startTime?.toISOString(),
      endTime: this.endTime?.toISOString(),
      exitCode: this.exitCode,
      workingDirectory: this.workingDirectory || '',
      shell: this.shell || '',
      error: this.error,
      duration:
        this.endTime && this.startTime
          ? this.endTime.getTime() - this.startTime.getTime()
          : undefined,
    }
  }

  /**
   * Get the process ID
   */
  getPid(): number | undefined {
    return this.pid
  }

  /**
   * Check if the command is running
   */
  isRunning(): boolean {
    return this.status === 'running'
  }

  /**
   * Execute a command and return a structured result
   */
  async executeWithResult(
    command: string,
    options?: ShellCommandOptions,
  ): Promise<ShellCommandResult> {
    const startTime = Date.now()

    // Execute the command and wait for completion
    await this.execute(command, { ...options, background: false })

    // Get the output (excluding system logs)
    const output = this.getOutputOnly()

    // Calculate duration
    const durationMs = Date.now() - startTime

    return {
      command,
      output,
      exitCode: this.exitCode || 0,
      durationMs,
      success: this.exitCode === 0,
      error: this.error,
    }
  }

  /**
   * Execute multiple commands in sequence
   */
  async executeSequence(
    commands: string[],
    options?: ShellCommandOptions & {
      separator?: string
      stopOnError?: boolean
    },
  ): Promise<void> {
    const separator = options?.separator || ' && '

    // Filter out empty commands
    const validCommands = commands.filter((cmd) => cmd.trim() !== '')

    if (validCommands.length === 0) {
      throw new Error('No valid commands provided')
    }

    // Combine commands with separator
    const combinedCommand = validCommands.join(separator)

    // Execute the combined command
    await this.execute(combinedCommand, options)
  }

  /**
   * Get formatted logs as a string
   */
  getFormattedLogs(includeSystem = false, lines?: number): string {
    let logs = this.logBuffer

    // Filter system logs if requested
    if (!includeSystem) {
      logs = logs.filter((log) => log.type !== 'system')
    }

    // Limit lines if requested
    if (lines && lines > 0) {
      const startIndex = Math.max(0, logs.length - lines)
      logs = logs.slice(startIndex)
    }

    // Format logs
    return logs.map((log) => `[${log.timestamp}] [${log.type}] ${log.text}`).join('')
  }

  /**
   * Get only stdout and stderr output as a string (no system logs)
   */
  getOutputOnly(): string {
    return this.logBuffer
      .filter((log) => log.type !== 'system')
      .map((log) => log.text)
      .join('')
  }
}
