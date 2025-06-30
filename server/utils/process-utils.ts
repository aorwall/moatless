import { type ChildProcess, exec, spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import { promisify } from 'node:util'

const execAsync = promisify(exec)

export interface ProcessOptions {
  cwd?: string
  env?: NodeJS.ProcessEnv
  timeout?: number
  shell?: boolean
  stdio?: 'pipe' | 'inherit' | 'ignore'
  maxBuffer?: number
}

export interface ProcessResult {
  stdout: string
  stderr: string
  exitCode: number
  signal?: string
  timedOut: boolean
}

export class ProcessManager extends EventEmitter {
  private activeProcesses: Map<string, ChildProcess> = new Map()

  async execute(
    command: string,
    args: string[] = [],
    options: ProcessOptions = {},
  ): Promise<ProcessResult> {
    const processId = randomUUID()

    return new Promise((resolve, reject) => {
      const child = spawn(command, args, {
        cwd: options.cwd,
        env: options.env || process.env,
        stdio: options.stdio || 'pipe',
        shell: options.shell || false,
      })

      this.activeProcesses.set(processId, child)

      let stdout = ''
      let stderr = ''
      let timedOut = false

      // Collect output
      if (child.stdout) {
        child.stdout.on('data', (data) => {
          stdout += data.toString()
          this.emit('stdout', processId, data.toString())
        })
      }

      if (child.stderr) {
        child.stderr.on('data', (data) => {
          stderr += data.toString()
          this.emit('stderr', processId, data.toString())
        })
      }

      // Handle process completion
      child.on('close', (code, signal) => {
        this.activeProcesses.delete(processId)

        resolve({
          stdout,
          stderr,
          exitCode: code || 0,
          signal: signal || undefined,
          timedOut,
        })
      })

      child.on('error', (error) => {
        this.activeProcesses.delete(processId)
        reject(error)
      })

      // Handle timeout
      if (options.timeout) {
        setTimeout(() => {
          if (this.activeProcesses.has(processId)) {
            timedOut = true
            child.kill('SIGTERM')

            // Force kill after 5 seconds
            setTimeout(() => {
              if (this.activeProcesses.has(processId)) {
                child.kill('SIGKILL')
              }
            }, 5000)
          }
        }, options.timeout)
      }
    })
  }

  async executeShell(command: string, options: ProcessOptions = {}): Promise<ProcessResult> {
    try {
      const result = await execAsync(command, {
        cwd: options.cwd,
        env: options.env || process.env,
        timeout: options.timeout,
        maxBuffer: options.maxBuffer || 1024 * 1024, // 1MB default
      })

      return {
        stdout: result.stdout,
        stderr: result.stderr,
        exitCode: 0,
        timedOut: false,
      }
    } catch (error: unknown) {
      const execError = error as {
        stdout?: string
        stderr?: string
        code?: number
        signal?: string
        killed?: boolean
      }
      return {
        stdout: execError.stdout || '',
        stderr: execError.stderr || '',
        exitCode: execError.code || 1,
        signal: execError.signal,
        timedOut: (execError.killed && execError.signal === 'SIGTERM') || false,
      }
    }
  }

  killProcess(processId: string, signal: NodeJS.Signals = 'SIGTERM'): boolean {
    const process = this.activeProcesses.get(processId)
    if (process && !process.killed) {
      process.kill(signal)
      return true
    }
    return false
  }

  killAllProcesses(signal: NodeJS.Signals = 'SIGTERM'): void {
    for (const [_processId, process] of this.activeProcesses) {
      if (!process.killed) {
        process.kill(signal)
      }
    }
  }

  getActiveProcessCount(): number {
    return this.activeProcesses.size
  }

  async cleanup(): Promise<void> {
    this.killAllProcesses('SIGTERM')

    // Wait for processes to exit gracefully
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Force kill any remaining processes
    this.killAllProcesses('SIGKILL')

    this.activeProcesses.clear()
  }
}

export const processManager = new ProcessManager()

// Cleanup on process exit
process.on('exit', () => {
  processManager.cleanup()
})

process.on('SIGINT', () => {
  processManager.cleanup().then(() => process.exit(0))
})

process.on('SIGTERM', () => {
  processManager.cleanup().then(() => process.exit(0))
})
