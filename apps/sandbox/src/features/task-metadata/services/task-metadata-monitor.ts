/**
 * TaskMetadataMonitor - Monitors task metadata updates via filesystem
 *
 * Follows the same pattern as PreviewProcessMonitor:
 * - Watches /opt/moatless/data/task-metadata.json for changes
 * - Emits 'task' events with subType 'metadataUpdated' when file changes
 * - Backend TaskEventHandler processes these events to update the task
 */

import { existsSync, readFileSync as fsReadFileSync } from 'node:fs'
import { watch } from 'node:fs/promises'
import type { Event } from '@moatless/types'
import type { SandboxConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'
import type { SandboxEventService } from '../../events/services/sandbox-event.service'

const DEFAULT_DATA_DIR = '/opt/moatless/data'

interface TaskMetadataFileContent {
  name?: string
  description?: string
  updatedAt?: string
}

export class TaskMetadataMonitor {
  private readonly monitorLogger = logger.child({ service: 'task-metadata-monitor' })
  private watchAbortController: AbortController | undefined
  private lastEmittedContent: string | undefined

  private readonly statusFile: string

  constructor(
    private readonly config: SandboxConfig,
    private readonly eventService?: SandboxEventService,
  ) {
    const dataDir = config.dataDir || DEFAULT_DATA_DIR
    this.statusFile = `${dataDir}/task-metadata.json`
    this.monitorLogger.info(
      { dataDir, statusFile: this.statusFile },
      'TaskMetadataMonitor initialized',
    )
  }

  /**
   * Read the current metadata from the status file
   */
  getMetadata(): TaskMetadataFileContent | null {
    try {
      if (!existsSync(this.statusFile)) {
        return null
      }
      const content = readFileSync(this.statusFile)
      return JSON.parse(content) as TaskMetadataFileContent
    } catch (error) {
      this.monitorLogger.warn({ err: error }, 'Failed to read metadata file')
      return null
    }
  }

  /**
   * Start watching the status file for changes
   */
  startWatching(): void {
    if (this.watchAbortController) {
      return // Already watching
    }

    this.watchAbortController = new AbortController()
    this.watchStatusFile(this.watchAbortController.signal).catch((error) => {
      if ((error as NodeJS.ErrnoException).name !== 'AbortError') {
        this.monitorLogger.error({ err: error }, 'Status file watcher error')
      }
    })

    this.monitorLogger.info('Started watching metadata file for changes')
  }

  /**
   * Stop watching for changes
   */
  stopWatching(): void {
    if (this.watchAbortController) {
      this.watchAbortController.abort()
      this.watchAbortController = undefined
      this.monitorLogger.info('Stopped watching metadata file')
    }
  }

  /**
   * Watch the status file and emit events when it changes
   */
  private async watchStatusFile(signal: AbortSignal): Promise<void> {
    // Wait for status file to appear
    while (!existsSync(this.statusFile)) {
      if (signal.aborted) {
        return
      }
      this.monitorLogger.debug('Metadata file does not exist yet, waiting...')
      await this.abortableDelay(2000, signal)
    }

    // Emit current status after file appears
    await this.checkAndEmitChanges()

    try {
      const watcher = watch(this.statusFile, { signal })

      for await (const _event of watcher) {
        try {
          await this.checkAndEmitChanges()
        } catch (readError) {
          this.monitorLogger.debug({ err: readError }, 'Error reading file during watch')
        }
      }
    } catch (error) {
      const errno = error as NodeJS.ErrnoException
      if (errno.name === 'AbortError') {
        return
      }
      if (errno.code === 'ENOENT') {
        // File was deleted after we started watching - restart
        this.monitorLogger.warn('Metadata file disappeared, restarting watch')
        return this.watchStatusFile(signal)
      }
      throw error
    }
  }

  /**
   * Check if content changed and emit event if so
   */
  private async checkAndEmitChanges(): Promise<void> {
    const metadata = this.getMetadata()
    if (!metadata) {
      return
    }

    const contentStr = JSON.stringify(metadata)
    if (contentStr !== this.lastEmittedContent) {
      await this.emitMetadataEvent(metadata)
      this.lastEmittedContent = contentStr
      this.monitorLogger.debug({ name: metadata.name }, 'Emitted metadata event after file change')
    }
  }

  /**
   * Emit a task metadata event via the event service
   */
  private async emitMetadataEvent(metadata: TaskMetadataFileContent): Promise<void> {
    if (!this.eventService) {
      return
    }

    try {
      // Cast via unknown since 'metadataUpdated' is not in the Event type's subType union
      // but is dynamically handled by the backend TaskEventHandler
      const event = {
        taskId: this.config.taskId,
        type: 'task',
        subType: 'metadataUpdated',
        payload: {
          name: metadata.name,
          description: metadata.description,
        },
        isError: false,
      } as unknown as Event

      await this.eventService.emitEvent(event)
      this.monitorLogger.debug({ name: metadata.name }, 'Emitted metadata event')
    } catch (emitError) {
      this.monitorLogger.warn({ err: emitError }, 'Failed to emit metadata event')
    }
  }

  /**
   * Delay that respects AbortSignal
   */
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
}

/**
 * Synchronous file read helper
 */
function readFileSync(filePath: string): string {
  return fsReadFileSync(filePath, 'utf-8')
}

// Module-scoped monitor instance
let monitorInstance: TaskMetadataMonitor | undefined

/**
 * Initialize the task metadata monitor
 */
export function initializeTaskMetadataMonitor(
  config: SandboxConfig,
  eventService?: SandboxEventService,
): TaskMetadataMonitor {
  monitorInstance = new TaskMetadataMonitor(config, eventService)
  return monitorInstance
}

/**
 * Get the monitor instance (or undefined if not initialized)
 */
export function getTaskMetadataMonitor(): TaskMetadataMonitor | undefined {
  return monitorInstance
}
