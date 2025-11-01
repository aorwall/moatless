import { randomUUID } from 'node:crypto'
import type { ServerEvent } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { EventService } from '../../events/services/event.service.js'
import { PreviewServerRepository } from '../db/preview-server.repository.js'
import type {
  PreviewServerLifecycleSink,
  PreviewServerRecord,
  PreviewServerSnapshot,
  PreviewServerUpdate,
  ServerStatusType,
} from '../types/index.js'

type PreviewServerRecordInput = PreviewServerSnapshot | PreviewServerUpdate

/**
 * PreviewRegistryService - Implements lifecycle sink pattern for preview servers
 *
 * Responsibilities:
 * - Register/update/remove preview server records
 * - Maintain in-memory cache + persistent storage
 * - Provide lookup for hostname resolution
 * - Implement PreviewServerLifecycleSink interface
 *
 * Used by:
 * - PreviewServer (lifecycle sink)
 * - RegistryTargetResolver (hostname resolution)
 * - HostnameProxyService (routing)
 */
@injectable()
export class PreviewRegistryService implements PreviewServerLifecycleSink {
  private readonly cache = new Map<string, PreviewServerRecord>()
  private readonly serviceLogger: typeof logger

  constructor(
    @inject(PreviewServerRepository) private readonly repository: PreviewServerRepository,
    @inject(EventService) private readonly eventService: EventService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-registry' })
  }

  /**
   * Assign optional fields from source to record
   * Handles undefined values and metadata merging
   */
  private assignOptionalFields(
    record: PreviewServerRecord,
    source: PreviewServerRecordInput | undefined,
  ): void {
    if (!source) {
      return
    }

    const optionalKeys: (keyof PreviewServerSnapshot)[] = [
      'port',
      'url',
      'proxyUrl',
      'hostname',
      'pid',
      'error',
      'metadata',
    ]

    const mutableRecord = record as unknown as Record<string, unknown>

    for (const key of optionalKeys) {
      if (Object.hasOwn(source, key)) {
        const value = source[key]
        if (value === undefined) {
          delete mutableRecord[key as string]
        } else if (key === 'metadata' && value) {
          mutableRecord[key as string] = {
            ...(value as Record<string, unknown>),
          }
        } else {
          mutableRecord[key as string] = value as unknown
        }
      }
    }
  }

  /**
   * Register a new preview server or update existing
   * Implements PreviewServerLifecycleSink.register
   */
  async register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void> {
    const existing = this.cache.get(serverId) ?? (await this.repository.get(serverId)) ?? undefined
    const now = new Date()

    const record: PreviewServerRecord = {
      serverId,
      status: snapshot.status ?? existing?.status ?? ('stopped' as ServerStatusType),
      inspectorEnabled: snapshot.inspectorEnabled ?? existing?.inspectorEnabled ?? true,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
    }

    this.assignOptionalFields(record, snapshot)

    await this.repository.save(record)
    this.cache.set(serverId, record)

    this.serviceLogger.debug('Preview server registered', { serverId, status: record.status })
  }

  /**
   * Update an existing preview server record
   * Implements PreviewServerLifecycleSink.update
   */
  async update(serverId: string, updates: PreviewServerUpdate): Promise<void> {
    const existing = this.cache.get(serverId) ?? (await this.repository.get(serverId)) ?? undefined

    if (!existing) {
      this.serviceLogger.warn('Cannot update missing preview server', { serverId })
      return
    }

    const record: PreviewServerRecord = {
      ...existing,
      serverId,
      updatedAt: new Date(),
    }

    if (updates.status) {
      record.status = updates.status
    }

    if (Object.hasOwn(updates, 'inspectorEnabled')) {
      record.inspectorEnabled =
        updates.inspectorEnabled !== undefined
          ? updates.inspectorEnabled
          : (record.inspectorEnabled ?? true)
    }

    this.assignOptionalFields(record, updates)

    await this.repository.save(record)
    this.cache.set(serverId, record)

    this.serviceLogger.debug('Preview server updated', { serverId, status: record.status })
  }

  /**
   * Remove a preview server record
   * Implements PreviewServerLifecycleSink.remove
   */
  async remove(serverId: string): Promise<void> {
    this.cache.delete(serverId)
    await this.repository.delete(serverId)
    this.serviceLogger.debug('Preview server removed', { serverId })
  }

  /**
   * Get a preview server record by ID
   * Used by RegistryTargetResolver for hostname resolution
   */
  async getRecord(serverId: string): Promise<PreviewServerRecord | null> {
    const cached = this.cache.get(serverId)
    if (cached) {
      return cached
    }

    const record = await this.repository.get(serverId)
    if (record) {
      this.cache.set(serverId, record)
    }

    return record
  }

  /**
   * List all preview server records
   */
  async listRecords(): Promise<PreviewServerRecord[]> {
    // Load all records from repository to ensure cache is up-to-date
    const records = await this.repository.list()

    // Update cache
    for (const record of records) {
      this.cache.set(record.serverId, record)
    }

    return records
  }

  /**
   * Optional status change callback
   * Implements PreviewServerLifecycleSink.onStatusChange
   * Emits server events through EventService
   */
  async onStatusChange?(
    serverId: string,
    statusInfo: {
      status: ServerStatusType
      port?: number
      pid?: number
      url?: string
      proxyUrl?: string
      error?: string
      uptimeSeconds?: number
    },
  ): Promise<void> {
    this.serviceLogger.info('Preview server status changed', { serverId, ...statusInfo })

    // serverId IS the taskId in preview server architecture
    const taskId = serverId

    // Map ServerStatusType to ServerEvent subType
    // ServerStatusType: 'stopped' | 'starting' | 'started' | 'failed'
    // ServerEvent subType: 'installing' | 'starting' | 'started' | 'stopped' | 'failed' | 'error'
    const subTypeMap: Record<ServerStatusType, ServerEvent['subType']> = {
      stopped: 'stopped',
      starting: 'starting',
      started: 'started',
      failed: 'failed',
    }

    // Get previous status from cache for previousStatus field
    const cachedRecord = this.cache.get(serverId)
    const previousStatus = cachedRecord?.status

    // Construct ServerEvent
    const serverEvent: ServerEvent = {
      eventId: randomUUID(),
      type: 'server',
      subType: subTypeMap[statusInfo.status],
      taskId,
      timestamp: new Date(),
      isError: statusInfo.status === 'failed',
      payload: {
        status: statusInfo.status,
        previousStatus,
        port: statusInfo.port,
        pid: statusInfo.pid,
        url: statusInfo.url,
        error: statusInfo.error,
        uptimeSeconds: statusInfo.uptimeSeconds,
      },
    }

    // Emit the event
    try {
      await this.eventService.emitEvent(serverEvent)
      this.serviceLogger.debug('Server event emitted', {
        serverId,
        eventId: serverEvent.eventId,
        subType: serverEvent.subType,
      })
    } catch (error) {
      this.serviceLogger.error('Failed to emit server event', error as Error, { serverId })
    }
  }
}
