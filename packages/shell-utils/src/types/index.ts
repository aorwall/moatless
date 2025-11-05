/**
 * Status of a shell command execution
 */
export type ShellCommandStatus = 'idle' | 'running' | 'completed' | 'failed' | 'killed'

/**
 * Options for executing a shell command (ShellExecutor)
 */
export interface ShellExecutorOptions {
  /** Working directory for command execution */
  workingDirectory?: string

  /** Environment variables to pass to the command */
  env?: Record<string, string>

  /** Timeout in milliseconds */
  timeout?: number

  /** Shell to use (defaults to /bin/bash on Unix, cmd.exe on Windows) */
  shell?: string

  /** Custom shell initialization commands (e.g., "source ~/.nvm/nvm.sh") */
  shellSetup?: string | string[]

  /** Callback when stdout data is received */
  onStdout?: (text: string, timestamp: Date) => void

  /** Callback when stderr data is received */
  onStderr?: (text: string, timestamp: Date) => void

  /** Callback when command starts (with PID and full command) */
  onStart?: (pid: number, command: string) => void

  /** Callback when command exits */
  onExit?: (code: number | null, signal: string | null) => void

  /** Callback when command encounters an error */
  onError?: (error: Error) => void
}

/**
 * Options for executing a shell command (ShellCommand with logging)
 */
export interface ShellCommandOptions {
  /** Working directory for command execution */
  workingDirectory?: string

  /** Environment variables to pass to the command */
  env?: Record<string, string>

  /** Timeout in milliseconds */
  timeout?: number

  /** Shell to use (defaults to /bin/bash on Unix, cmd.exe on Windows) */
  shell?: string

  /** Custom shell initialization commands (e.g., "source ~/.nvm/nvm.sh") */
  shellSetup?: string | string[]

  /** Path to log file for command output */
  logFile?: string
}

/**
 * Log event from shell command execution
 */
export interface ShellCommandLogEvent {
  /** Type of log event */
  type: 'stdout' | 'stderr' | 'system'

  /** ISO timestamp of the event */
  timestamp: string

  /** Log text content */
  text: string
}

/**
 * Result of a shell command execution
 */
export interface ShellCommandResult {
  command: string
  output: string
  exitCode: number
  durationMs: number
  success: boolean
  error?: string
}
