import { type ChildProcess, spawn } from 'node:child_process'
import type { CommandExecutionEvent } from '@moatless/types'

export interface CommandExecutionOptions {
  cwd: string
  env?: Record<string, string>
  timeout?: number
  shell?: string
}

export interface SpawnResult {
  child: ChildProcess
  shellCommand: string
  startTime: number
  timeoutHandle: NodeJS.Timeout
}

/**
 * Synchronously spawn a command and return the child process with setup info
 * This allows immediate access to the process and PID while monitoring can happen separately
 */
export function spawnCommand(command: string, options: CommandExecutionOptions): SpawnResult {
  const { cwd, env = {}, timeout = 30000, shell } = options
  const startTime = Date.now()

  // Create the shell command with improved environment setup
  const shellCommand =
    process.platform === 'win32'
      ? command
      : `export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"; ${command}`

  const shellExecutable =
    shell || (process.platform === 'win32' ? 'cmd.exe' : process.env.SHELL || '/bin/zsh')
  const shellArgs = process.platform === 'win32' ? ['/c', shellCommand] : ['-c', shellCommand]

  // Use spawn for streaming output
  const child = spawn(shellExecutable, shellArgs, {
    cwd,
    env: { ...process.env, ...env },
    stdio: ['ignore', 'pipe', 'pipe'], // Explicitly set stdio
  })

  // Set up timeout
  const timeoutHandle = setTimeout(() => {
    child.kill('SIGTERM')
  }, timeout)

  return {
    child,
    shellCommand,
    startTime,
    timeoutHandle,
  }
}

/**
 * Monitor an already spawned command for output and completion events
 * This allows monitoring to happen asynchronously after process is spawned
 */
export async function* monitorCommandOutput(
  spawnResult: SpawnResult,
  command: string,
): AsyncGenerator<CommandExecutionEvent> {
  const { child, startTime, timeoutHandle } = spawnResult

  // Emit start event immediately
  yield {
    type: 'start',
    data: {
      timestamp: new Date().toISOString(),
      command,
      pid: child.pid,
    },
  }

  let outputBuffer = ''
  let errorBuffer = ''
  const timedOut = false
  let spawnFailed = false
  let spawnErrorMessage = ''

  // Handle spawn errors
  child.on('error', (error: Error) => {
    spawnFailed = true
    spawnErrorMessage = error.message
  })

  // Handle timeout cleanup
  const originalTimeout = timeoutHandle
  timeoutHandle.unref() // Don't keep process alive

  // Set up arrays to collect stderr chunks
  const stderrChunks: Array<{ text: string; timestamp: string }> = []

  // Collect stderr
  if (child.stderr) {
    child.stderr.on('data', (chunk) => {
      const text = chunk.toString()
      errorBuffer += text
      stderrChunks.push({
        text,
        timestamp: new Date().toISOString(),
      })
    })
  }

  // Monitor for timeout
  originalTimeout.unref() // Don't keep process alive

  // Stream stdout and periodically check for stderr
  if (child.stdout) {
    for await (const chunk of child.stdout) {
      const text = chunk.toString()
      outputBuffer += text

      // Yield stdout
      yield {
        type: 'output',
        data: {
          timestamp: new Date().toISOString(),
          text,
          outputType: 'stdout',
        },
      }

      // Yield any accumulated stderr
      while (stderrChunks.length > 0) {
        const stderrChunk = stderrChunks.shift()!
        yield {
          type: 'output',
          data: {
            timestamp: stderrChunk.timestamp,
            text: stderrChunk.text,
            outputType: 'stderr',
          },
        }
      }
    }
  }

  // Wait for process to exit
  const exitCode = await new Promise<number>((resolve) => {
    child.on('exit', (code) => {
      clearTimeout(originalTimeout)
      resolve(code || 0)
    })
    child.on('error', (_error) => {
      clearTimeout(originalTimeout)
      resolve(1)
    })
  })

  // If spawn failed, yield error immediately
  if (spawnFailed) {
    yield {
      type: 'complete',
      data: {
        timestamp: new Date().toISOString(),
        command,
        exitCode: 1,
        duration: Date.now() - startTime,
        error: `Failed to spawn process: ${spawnErrorMessage}`,
      },
    }
    return
  }

  // Yield any remaining stderr
  while (stderrChunks.length > 0) {
    const stderrChunk = stderrChunks.shift()!
    yield {
      type: 'output',
      data: {
        timestamp: stderrChunk.timestamp,
        text: stderrChunk.text,
        outputType: 'stderr',
      },
    }
  }

  const duration = Date.now() - startTime

  // Emit complete event
  yield {
    type: 'complete',
    data: {
      timestamp: new Date().toISOString(),
      command,
      exitCode,
      duration,
      error: timedOut
        ? 'Command timed out'
        : exitCode !== 0 && errorBuffer
          ? errorBuffer
          : undefined,
    },
  }
}

/**
 * Execute a command and yield execution events for streaming and logging
 * This is a shared utility that can be consumed by different components
 *
 * This function now uses the new split approach internally for backwards compatibility
 */
export async function* executeCommandWithStream(
  command: string,
  options: CommandExecutionOptions,
): AsyncGenerator<CommandExecutionEvent> {
  // Use the new split approach internally
  const spawnResult = spawnCommand(command, options)

  // Monitor the spawned process
  yield* monitorCommandOutput(spawnResult, command)
}
