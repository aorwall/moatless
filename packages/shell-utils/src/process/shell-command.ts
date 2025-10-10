import { type ChildProcess, spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { getSafeSystemEnvironment } from '../system/environment-utils.js'
import type {
  ShellCommandLogEvent,
  ShellCommandOptions,
  ShellCommandResult,
  ShellCommandStatus,
} from '../types/index.js'
import { treeKill } from './tree-kill.js'

/**
 * Shell command executor with log buffering, streaming, and file persistence.
 * Unified implementation with process management and logging capabilities.
 *
 * Features:
 * - Process spawning and lifecycle management
 * - Shell setup commands (e.g., NVM initialization)
 * - Log event buffering (stdout, stderr, system events)
 * - Log file persistence
 * - Log streaming via async generator
 * - Formatted log output
 * - Command sequencing
 *
 * Note: Throws errors on log persistence failure - caller should handle/log them.
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
  private timeoutHandle: NodeJS.Timeout | undefined
  private hasExecuted = false
  private logBuffer: ShellCommandLogEvent[] = []
  private logFile: string | undefined

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
    this.logFile = options?.logFile

    const workingDirectory = options?.workingDirectory || process.cwd()
    const shell = options?.shell || (process.platform === 'win32' ? 'cmd.exe' : '/bin/bash')
    const timeout = options?.timeout
    const background = options?.background ?? true

    // Build environment: safe system vars + workspace-specific vars
    const processEnv = {
      ...getSafeSystemEnvironment(),
      ...options?.env,
    }

    // Store command info
    this.command = command
    this.workingDirectory = workingDirectory
    this.shell = shell
    this.status = 'running'
    this.startTime = new Date()

    // Build shell command with optional setup
    const shellCommand = this.buildShellCommand(command, options?.shellSetup)
    const shellArgs = process.platform === 'win32' ? ['/c', shellCommand] : ['-c', shellCommand]

    // Spawn the process
    this.process = spawn(shell, shellArgs, {
      cwd: workingDirectory,
      env: processEnv,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    this.pid = this.process.pid

    // Close stdin pipe to signal EOF to child process
    if (this.process.stdin) {
      this.process.stdin.end()
    }

    // Log start event
    this.handleStart(this.pid, command)

    // Set up timeout if specified
    if (timeout) {
      this.timeoutHandle = setTimeout(() => {
        this.kill().catch(() => {
          // Ignore kill errors in timeout handler
        })
        this.status = 'killed'
        this.error = 'Command timed out'
      }, timeout)
    }

    // Set up output streaming
    this.setupOutputStreaming()

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

  /**
   * Build the shell command with optional setup
   */
  private buildShellCommand(command: string, shellSetup?: string | string[]): string {
    if (!shellSetup) {
      return command
    }

    const setupCommands = Array.isArray(shellSetup) ? shellSetup : [shellSetup]
    const setup = setupCommands.join('; ')

    return `${setup}; ${command}`
  }

  /**
   * Set up output streaming
   */
  private setupOutputStreaming(): void {
    if (!this.process) {
      return
    }

    // Stream stdout
    if (this.process.stdout) {
      this.process.stdout.on('data', (chunk) => {
        const text = chunk.toString()
        const timestamp = new Date()
        this.handleStdout(text, timestamp)
      })
    }

    // Stream stderr
    if (this.process.stderr) {
      this.process.stderr.on('data', (chunk) => {
        const text = chunk.toString()
        const timestamp = new Date()
        this.handleStderr(text, timestamp)
      })
    }
  }

  /**
   * Handle process exit
   */
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

    // Log exit event
    this.handleExit(code, signal)
  }

  /**
   * Handle process error
   */
  private handleProcessError(error: Error): void {
    this.status = 'failed'
    this.error = error.message
    this.endTime = new Date()

    // Log error event
    this.handleError(error)
  }

  /**
   * Handle stdout callback
   */
  private handleStdout(text: string, timestamp: Date): void {
    const logEvent: ShellCommandLogEvent = {
      type: 'stdout',
      timestamp: timestamp.toISOString(),
      text,
    }
    this.logBuffer.push(logEvent)
    this.persistLog(logEvent)
  }

  /**
   * Handle stderr callback
   */
  private handleStderr(text: string, timestamp: Date): void {
    const logEvent: ShellCommandLogEvent = {
      type: 'stderr',
      timestamp: timestamp.toISOString(),
      text,
    }
    this.logBuffer.push(logEvent)
    this.persistLog(logEvent)
  }

  /**
   * Handle start callback
   */
  private handleStart(pid: number | undefined, command: string): void {
    const startEvent: ShellCommandLogEvent = {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Command started: ${command}\\nPID: ${pid || 'unknown'}\\n`,
    }
    this.logBuffer.push(startEvent)
    this.persistLog(startEvent)
  }

  /**
   * Handle exit callback
   */
  private handleExit(code: number | null, signal: string | null): void {
    const endEvent: ShellCommandLogEvent = {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Command ${this.status}: exit code ${code}, signal ${signal}\\n`,
    }
    this.logBuffer.push(endEvent)
    this.persistLog(endEvent)
  }

  /**
   * Handle error callback
   */
  private handleError(error: Error): void {
    const errorEvent: ShellCommandLogEvent = {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Error: ${error.message}\\n`,
    }
    this.logBuffer.push(errorEvent)
    this.persistLog(errorEvent)
  }

  /**
   * Persist log to file
   * Throws on error - caller should handle/log errors
   */
  private persistLog(logEvent: ShellCommandLogEvent): void {
    if (!this.logFile) {
      return
    }
    // Store logFile in local variable for use in async IIFE
    const logFile = this.logFile
    // Use async IIFE to handle async operations
    ;(async () => {
      const dir = path.dirname(logFile)
      await fs.mkdir(dir, { recursive: true })

      const logLine = `[${logEvent.timestamp}] [${logEvent.type}] ${logEvent.text}`
      await fs.appendFile(logFile, logLine)
    })().catch((error) => {
      // Re-throw for caller to handle
      throw new Error(`Failed to persist log to ${logFile}: ${error.message}`)
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
      return await new Promise<boolean>((resolve) => {
        treeKill(pid, 'SIGTERM', (err) => {
          if (err) {
            // If SIGTERM fails, force kill with SIGKILL
            treeKill(pid, 'SIGKILL', (killErr) => {
              resolve(!killErr)
            })
          } else {
            resolve(true)
          }
        })
      })
    } catch {
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
    if (this.isRunning()) {
      let lastIndex = this.logBuffer.length

      // Poll for new logs
      while (this.isRunning()) {
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
   * Get the process ID
   */
  getPid(): number | undefined {
    return this.pid
  }

  /**
   * Get the exit code
   */
  getExitCode(): number | undefined {
    return this.exitCode
  }

  /**
   * Get error message if command failed
   */
  getError(): string | undefined {
    return this.error
  }

  /**
   * Get command duration in milliseconds
   */
  getDuration(): number | undefined {
    if (this.endTime && this.startTime) {
      return this.endTime.getTime() - this.startTime.getTime()
    }
    return undefined
  }

  /**
   * Get command start time (ISO string)
   */
  getStartTime(): string | undefined {
    return this.startTime?.toISOString()
  }

  /**
   * Get command end time (ISO string)
   */
  getEndTime(): string | undefined {
    return this.endTime?.toISOString()
  }

  /**
   * Get working directory
   */
  getWorkingDirectory(): string | undefined {
    return this.workingDirectory
  }

  /**
   * Get shell used
   */
  getShell(): string | undefined {
    return this.shell
  }

  /**
   * Get executed command
   */
  getCommand(): string | undefined {
    return this.command
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
    // Execute with our wrapper
    await this.execute(command, { ...options, background: false })

    // Get the output from the executor
    const output = this.getOutputOnly()

    const result: ShellCommandResult = {
      command,
      output,
      exitCode: this.getExitCode() || 0,
      durationMs: this.getDuration() || 0,
      success: this.getExitCode() === 0,
    }

    const error = this.getError()
    if (error !== undefined) {
      result.error = error
    }

    return result
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
