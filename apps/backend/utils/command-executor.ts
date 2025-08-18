import { spawn } from 'node:child_process'
import type { CommandExecutionEvent } from '@moatless/types'

export interface CommandExecutionOptions {
  cwd: string
  env?: Record<string, string>
  timeout?: number
  shell?: string
}

/**
 * Execute a command and yield execution events for streaming and logging
 * This is a shared utility that can be consumed by different components
 */
export async function* executeCommandWithStream(
  command: string,
  options: CommandExecutionOptions,
): AsyncGenerator<CommandExecutionEvent> {
  const { cwd, env = {}, timeout = 30000, shell } = options
  const startTime = Date.now()

  // Emit start event
  yield {
    type: 'start',
    data: {
      timestamp: new Date().toISOString(),
      command,
    },
  }

  // Create the shell command with environment setup
  const shellCommand =
    process.platform === 'win32'
      ? command
      : `source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null || true; ${command}`

  const shellExecutable =
    shell || (process.platform === 'win32' ? 'cmd.exe' : process.env.SHELL || '/bin/zsh')
  const shellArgs = process.platform === 'win32' ? ['/c', shellCommand] : ['-c', shellCommand]

  // Use spawn for streaming output
  const child = spawn(shellExecutable, shellArgs, {
    cwd,
    env: { ...process.env, ...env },
    timeout,
  })

  let outputBuffer = ''
  let errorBuffer = ''
  let timedOut = false

  // Set up timeout
  const timeoutHandle = setTimeout(() => {
    timedOut = true
    child.kill('SIGTERM')
  }, timeout)

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
      clearTimeout(timeoutHandle)
      resolve(code || 0)
    })
    child.on('error', (_error) => {
      clearTimeout(timeoutHandle)
      resolve(1)
    })
  })

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
