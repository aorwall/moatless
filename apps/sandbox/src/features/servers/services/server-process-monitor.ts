/**
 * ServerProcessMonitor - Generic monitor for standalone server processes via filesystem
 *
 * Monitors server processes managed externally (by shell scripts or agents).
 * Watches filesystem artifacts (PID file, status file, log file) and reports status.
 * Parameterized by server name — file paths become `{name}-server.{pid,status.json,log,config.json}`.
 */

import { createReadStream } from 'node:fs'
import { access, readFile, stat, watch, writeFile } from 'node:fs/promises'
import { Socket } from 'node:net'
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
  private portPollTimer: ReturnType<typeof setInterval> | undefined

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
  getStatus(): Promise<PreviewServerStatus> {
    return this.readStatusFile(0)
  }

  private async readStatusFile(attempt: number): Promise<PreviewServerStatus> {
    try {
      if (!(await pathExists(this.statusFile))) {
        return { status: 'stopped' }
      }

      const content = await readFile(this.statusFile, 'utf-8')
      const parsed = JSON.parse(content) as StatusFileContent

      if (
        (parsed.status === 'started' || parsed.status === 'starting') &&
        !this.isProcessAlive(parsed.pid)
      ) {
        // Only mark as failed if a PID was actually recorded (meaning the
        // process was launched and then died). If no PID exists yet, the
        // entrypoint wrote "starting" but hasn't launched the server process
        // — keep reporting "starting" so the monitor continues waiting.
        if (parsed.status === 'starting') {
          if (parsed.pid) {
            return {
              status: 'failed',
              error: 'Process exited before becoming responsive',
            }
          }
          return { status: 'starting' }
        }
        return { status: 'stopped' }
      }

      const port = parsed.port || (await this.getConfiguredPort())

      return {
        status: parsed.status,
        pid: parsed.pid,
        internalUrl: parsed.status === 'started' ? `http://localhost:${port}` : undefined,
        error: parsed.error,
      }
    } catch (error) {
      if (error instanceof SyntaxError && attempt < 2) {
        this.monitorLogger.info(
          { attempt: attempt + 1 },
          'Status file has incomplete JSON, retrying',
        )
        await new Promise((r) => setTimeout(r, 50))
        return this.readStatusFile(attempt + 1)
      }
      this.monitorLogger.warn({ err: error }, 'Failed to read status file')
      return { status: 'stopped' }
    }
  }

  /**
   * Check if the server process is running
   */
  async isRunning(): Promise<boolean> {
    const pid = await this.readPid()
    if (!pid) {
      return false
    }
    return this.isProcessAlive(pid)
  }

  /**
   * Get the configured port from the config file
   */
  async getConfiguredPort(): Promise<number> {
    try {
      if (!(await pathExists(this.configFile))) {
        return this.getDefaultPort()
      }
      const content = await readFile(this.configFile, 'utf-8')
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
  async getLogs(lines = 100): Promise<string> {
    try {
      if (!(await pathExists(this.logFile))) {
        return ''
      }

      const content = await readFile(this.logFile, 'utf-8')
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

    if (!(await pathExists(this.logFile))) {
      yield {
        type: 'error',
        data: {
          error: 'Log file not found',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    const existingContent = await this.getLogs(500)
    if (existingContent) {
      yield {
        type: 'chunk',
        data: {
          text: existingContent,
          timestamp: new Date().toISOString(),
        },
      }
    }

    let lastSize = (await pathExists(this.logFile)) ? (await stat(this.logFile)).size : 0
    const abortController = new AbortController()

    try {
      const watcher = watch(this.logFile, { signal: abortController.signal })

      for await (const _event of watcher) {
        try {
          const currentSize = (await stat(this.logFile)).size
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
    this.stopPortPolling()
    if (this.watchAbortController) {
      this.watchAbortController.abort()
      this.watchAbortController = undefined
      this.monitorLogger.info('Stopped watching status file')
    }
  }

  private async watchStatusFile(signal: AbortSignal): Promise<void> {
    const initialStatus = await this.getStatus()
    if (initialStatus.status !== 'stopped') {
      await this.emitServerEvent(initialStatus.status)
      this.lastEmittedStatus = initialStatus.status
    }

    while (!(await pathExists(this.statusFile))) {
      if (signal.aborted) {
        return
      }
      this.monitorLogger.debug('Status file does not exist yet, waiting...')
      await this.abortableDelay(2000, signal)
    }

    const currentStatus = await this.getStatus()
    if (currentStatus.status !== this.lastEmittedStatus) {
      await this.emitServerEvent(currentStatus.status, currentStatus.error)
      this.lastEmittedStatus = currentStatus.status
      this.monitorLogger.debug(
        { status: currentStatus.status },
        'Emitted status after file appeared',
      )
    }

    if (currentStatus.status === 'starting') {
      this.startPortPolling()
    }

    try {
      const watcher = watch(this.statusFile, { signal })

      for await (const _event of watcher) {
        await this.handleStatusChange()
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

  private async handleStatusChange(): Promise<void> {
    try {
      const status = await this.getStatus()

      if (status.status === 'starting' && !this.portPollTimer) {
        this.startPortPolling()
      } else if (status.status !== 'starting') {
        this.stopPortPolling()
      }

      if (status.status !== this.lastEmittedStatus) {
        await this.emitServerEvent(status.status, status.error)
        this.lastEmittedStatus = status.status
      }
    } catch (readError) {
      this.monitorLogger.debug({ err: readError }, 'Error reading status during watch')
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

  private checkPort(port: number): Promise<boolean> {
    // Use TCP connect instead of HTTP fetch to avoid triggering expensive route
    // compilation in frameworks like Next.js Turbopack (where GET / can take >2s
    // to compile on first request, causing repeated fetch timeouts).
    return new Promise<boolean>((resolve) => {
      const socket = new Socket()
      socket.setTimeout(2000)
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('timeout', () => {
        socket.destroy()
        resolve(false)
      })
      socket.once('error', () => {
        socket.destroy()
        resolve(false)
      })
      socket.connect(port, '127.0.0.1')
    })
  }

  private async updateStatusFile(
    status: ServerStatusType,
    extra?: { error?: string },
  ): Promise<void> {
    try {
      let currentContent: StatusFileContent = {} as StatusFileContent
      if (await pathExists(this.statusFile)) {
        const raw = await readFile(this.statusFile, 'utf-8')
        currentContent = JSON.parse(raw) as StatusFileContent
      }

      const updated: StatusFileContent = {
        ...currentContent,
        status,
        startedAt: new Date().toISOString(),
      }
      if (extra?.error) {
        updated.error = extra.error
      }

      await writeFile(this.statusFile, JSON.stringify(updated))
      this.monitorLogger.debug({ status }, 'Updated status file')
    } catch (error) {
      this.monitorLogger.warn({ err: error }, 'Failed to update status file')
    }
  }

  private async startPortPolling(): Promise<void> {
    if (this.portPollTimer) {
      return
    }

    const port = await this.getConfiguredPort()
    this.monitorLogger.info({ port }, 'Starting port polling for readiness')

    this.portPollTimer = setInterval(async () => {
      const status = await this.getStatus()

      // If no longer in "starting" state, stop polling
      if (status.status !== 'starting') {
        this.stopPortPolling()
        return
      }

      // Check if process died — but only if a PID was recorded.
      // If no PID file exists, the server process hasn't been launched
      // yet (entrypoint is still running setup commands).
      const pid = await this.readPid()
      if (pid !== undefined && !this.isProcessAlive(pid)) {
        this.monitorLogger.info('Process died while starting, marking as failed')
        await this.updateStatusFile('failed', {
          error: 'Process exited before becoming responsive',
        })
        await this.emitServerEvent('failed', 'Process exited before becoming responsive')
        this.lastEmittedStatus = 'failed'
        this.stopPortPolling()
        return
      }

      // Check if port is reachable
      const reachable = await this.checkPort(port)
      if (reachable) {
        this.monitorLogger.info({ port }, 'Port is reachable, marking as started')
        await this.updateStatusFile('started')
        await this.emitServerEvent('started')
        this.lastEmittedStatus = 'started'
        this.stopPortPolling()
      }
    }, 2000)
  }

  private stopPortPolling(): void {
    if (this.portPollTimer) {
      clearInterval(this.portPollTimer)
      this.portPollTimer = undefined
      this.monitorLogger.debug('Stopped port polling')
    }
  }

  private async readPid(): Promise<number | undefined> {
    try {
      if (!(await pathExists(this.pidFile))) {
        return undefined
      }
      const content = (await readFile(this.pidFile, 'utf-8')).trim()
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

async function pathExists(p: string): Promise<boolean> {
  try {
    await access(p)
    return true
  } catch {
    return false
  }
}
