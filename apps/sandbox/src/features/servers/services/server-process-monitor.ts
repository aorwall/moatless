/**
 * ServerProcessMonitor - Generic monitor for standalone server processes via filesystem
 *
 * Monitors server processes managed externally (by shell scripts or agents).
 * Watches filesystem artifacts (PID file, status file, log file) and reports status.
 * Parameterized by server name — file paths become `{name}-server.{pid,status.json,log,config.json}`.
 */

import { createReadStream, existsSync, readFileSync as fsReadFileSync, statSync } from 'node:fs'
import { watch } from 'node:fs/promises'
import { createInterface } from 'node:readline'
import type { Event } from '@moatless/types'
import type { SandboxConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'
import type { SandboxEventService } from '../../events/services/sandbox-event.service'
import type { PreviewServerStatus, ServerStatusType } from '../../preview/types/index'

const DEFAULT_DATA_DIR = '/opt/moatless/data'

interface StatusFileContent {
  status: ServerStatusType
  port?: number
  pid?: number
  startedAt?: string
  error?: string
}

export interface ServerConfig {
  startCommands?: string[]
  /** @deprecated Use startCommands instead */
  serverCommands?: string[]
  port: number
  startupTimeoutSeconds: number
  workspacePath: string
}

interface LogStreamEvent {
  type: 'chunk' | 'error' | 'complete' | 'reset'
  data: {
    text?: string
    error?: string
    timestamp: string
  }
}

export class ServerProcessMonitor {
  private readonly monitorLogger: ReturnType<typeof logger.child>
  private watchAbortController: AbortController | undefined
  private lastEmittedStatus: ServerStatusType | undefined

  readonly name: string
  private readonly pidFile: string
  private readonly logFile: string
  private readonly statusFile: string
  private readonly configFile: string

  constructor(
    name: string,
    private readonly config: SandboxConfig,
    private readonly eventService?: SandboxEventService,
  ) {
    this.name = name
    this.monitorLogger = logger.child({ service: `${name}-process-monitor` })
    const dataDir = config.dataDir || DEFAULT_DATA_DIR
    this.pidFile = `${dataDir}/${name}-server.pid`
    this.logFile = `${dataDir}/${name}-server.log`
    this.statusFile = `${dataDir}/${name}-server.status.json`
    this.configFile = `${dataDir}/${name}-server.config.json`
    this.monitorLogger.info({ dataDir, name }, 'ServerProcessMonitor initialized')
  }

  /**
   * Read the current server status from the status file
   */
  getStatus(): PreviewServerStatus {
    try {
      if (!existsSync(this.statusFile)) {
        return { status: 'stopped' }
      }

      const content = readFileSync(this.statusFile)
      const parsed = JSON.parse(content) as StatusFileContent

      if (
        (parsed.status === 'started' || parsed.status === 'starting') &&
        !this.isProcessAlive(parsed.pid)
      ) {
        return { status: 'stopped' }
      }

      const port = parsed.port || this.getConfiguredPort()

      return {
        status: parsed.status,
        pid: parsed.pid,
        internalUrl: parsed.status === 'started' ? `http://localhost:${port}` : undefined,
        error: parsed.error,
      }
    } catch (error) {
      this.monitorLogger.warn({ err: error }, 'Failed to read status file')
      return { status: 'stopped' }
    }
  }

  /**
   * Check if the server process is running
   */
  isRunning(): boolean {
    const pid = this.readPid()
    if (!pid) {
      return false
    }
    return this.isProcessAlive(pid)
  }

  /**
   * Get the configured port from the config file
   */
  getConfiguredPort(): number {
    try {
      if (!existsSync(this.configFile)) {
        return this.getDefaultPort()
      }
      const content = readFileSync(this.configFile)
      const config = JSON.parse(content) as ServerConfig
      return config.port || this.getDefaultPort()
    } catch {
      return this.getDefaultPort()
    }
  }

  /**
   * Default port fallback when no config file exists
   */
  private getDefaultPort(): number {
    return 3000
  }

  /**
   * Read server logs (tail -n)
   */
  getLogs(lines = 100): string {
    try {
      if (!existsSync(this.logFile)) {
        return ''
      }

      const content = readFileSync(this.logFile)
      const allLines = content.split('\n')
      return allLines.slice(-lines).join('\n')
    } catch (error) {
      this.monitorLogger.warn({ err: error }, 'Failed to read log file')
      return ''
    }
  }

  /**
   * Stream log file changes (like tail -f)
   */
  async *streamLogs(): AsyncGenerator<LogStreamEvent> {
    yield {
      type: 'reset',
      data: { timestamp: new Date().toISOString() },
    }

    if (!existsSync(this.logFile)) {
      yield {
        type: 'error',
        data: {
          error: 'Log file not found',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    const existingContent = this.getLogs(500)
    if (existingContent) {
      yield {
        type: 'chunk',
        data: {
          text: existingContent,
          timestamp: new Date().toISOString(),
        },
      }
    }

    let lastSize = existsSync(this.logFile) ? statSync(this.logFile).size : 0
    const abortController = new AbortController()

    try {
      const watcher = watch(this.logFile, { signal: abortController.signal })

      for await (const _event of watcher) {
        try {
          const currentSize = statSync(this.logFile).size
          if (currentSize > lastSize) {
            const stream = createReadStream(this.logFile, {
              start: lastSize,
              end: currentSize - 1,
            })
            const rl = createInterface({ input: stream })

            for await (const line of rl) {
              yield {
                type: 'chunk',
                data: {
                  text: `${line}\n`,
                  timestamp: new Date().toISOString(),
                },
              }
            }

            lastSize = currentSize
          } else if (currentSize < lastSize) {
            lastSize = 0
            yield {
              type: 'reset',
              data: { timestamp: new Date().toISOString() },
            }
          }
        } catch (readError) {
          this.monitorLogger.debug({ err: readError }, 'Error reading log file during stream')
        }
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).name !== 'AbortError') {
        yield {
          type: 'error',
          data: {
            error: error instanceof Error ? error.message : String(error),
            timestamp: new Date().toISOString(),
          },
        }
      }
    }
  }

  /**
   * Start watching the status file for changes and emit events
   */
  startWatching(): void {
    if (this.watchAbortController) {
      return
    }

    this.watchAbortController = new AbortController()
    this.watchStatusFile(this.watchAbortController.signal).catch((error) => {
      if ((error as NodeJS.ErrnoException).name !== 'AbortError') {
        this.monitorLogger.error({ err: error }, 'Status file watcher error')
      }
    })

    this.monitorLogger.info('Started watching status file for changes')
  }

  /**
   * Stop watching for status changes
   */
  stopWatching(): void {
    if (this.watchAbortController) {
      this.watchAbortController.abort()
      this.watchAbortController = undefined
      this.monitorLogger.info('Stopped watching status file')
    }
  }

  private async watchStatusFile(signal: AbortSignal): Promise<void> {
    const initialStatus = this.getStatus()
    if (initialStatus.status !== 'stopped') {
      await this.emitServerEvent(initialStatus.status)
      this.lastEmittedStatus = initialStatus.status
    }

    while (!existsSync(this.statusFile)) {
      if (signal.aborted) {
        return
      }
      this.monitorLogger.debug('Status file does not exist yet, waiting...')
      await this.abortableDelay(2000, signal)
    }

    const currentStatus = this.getStatus()
    if (currentStatus.status !== this.lastEmittedStatus) {
      await this.emitServerEvent(currentStatus.status, currentStatus.error)
      this.lastEmittedStatus = currentStatus.status
      this.monitorLogger.debug(
        { status: currentStatus.status },
        'Emitted status after file appeared',
      )
    }

    try {
      const watcher = watch(this.statusFile, { signal })

      for await (const _event of watcher) {
        try {
          const status = this.getStatus()
          if (status.status !== this.lastEmittedStatus) {
            await this.emitServerEvent(status.status, status.error)
            this.lastEmittedStatus = status.status
          }
        } catch (readError) {
          this.monitorLogger.debug({ err: readError }, 'Error reading status during watch')
        }
      }
    } catch (error) {
      const errno = error as NodeJS.ErrnoException
      if (errno.name === 'AbortError') {
        return
      }
      if (errno.code === 'ENOENT') {
        this.monitorLogger.warn('Status file disappeared, restarting watch')
        return this.watchStatusFile(signal)
      }
      throw error
    }
  }

  private abortableDelay(ms: number, signal: AbortSignal): Promise<void> {
    if (signal.aborted) {
      return Promise.reject(new DOMException('Aborted', 'AbortError'))
    }
    return new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(resolve, ms)
      signal.addEventListener(
        'abort',
        () => {
          clearTimeout(timeout)
          reject(new DOMException('Aborted', 'AbortError'))
        },
        { once: true },
      )
    })
  }

  private async emitServerEvent(status: ServerStatusType, error?: string): Promise<void> {
    if (!this.eventService) {
      return
    }

    try {
      const event = {
        taskId: this.config.taskId,
        type: 'server' as const,
        subType: status,
        payload: {
          serverName: this.name,
          status,
          ...(error && { error }),
        },
        isError: status === 'failed',
      } as unknown as Event

      await this.eventService.emitEvent(event)
      this.monitorLogger.debug({ status }, 'Emitted server event')
    } catch (emitError) {
      this.monitorLogger.warn({ err: emitError, status }, 'Failed to emit server event')
    }
  }

  private readPid(): number | undefined {
    try {
      if (!existsSync(this.pidFile)) {
        return undefined
      }
      const content = readFileSync(this.pidFile).trim()
      const pid = Number.parseInt(content, 10)
      return Number.isNaN(pid) ? undefined : pid
    } catch {
      return undefined
    }
  }

  private isProcessAlive(pid: number | undefined): boolean {
    if (!pid) {
      return false
    }
    try {
      process.kill(pid, 0)
      return true
    } catch {
      return false
    }
  }
}

function readFileSync(filePath: string): string {
  return fsReadFileSync(filePath, 'utf-8')
}
