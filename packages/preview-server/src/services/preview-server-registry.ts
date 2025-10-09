import type { Logger } from '../types/interfaces.js'
import type {
  PreviewServerRecord as ExternalPreviewServerRecord,
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerUpdate,
} from '../types/registry.js'
import type { ServerStatusType } from '../types/status.js'

export interface PreviewServerRegistryStore {
  save(record: PreviewServerRecord): Promise<void>
  get(serverId: string): Promise<PreviewServerRecord | null>
  delete(serverId: string): Promise<void>
  list(): Promise<PreviewServerRecord[]>
}

export interface PreviewServerRecord extends ExternalPreviewServerRecord {}

type PreviewServerRecordInput = PreviewServerSnapshot | PreviewServerUpdate

export class InMemoryPreviewServerRegistryStore implements PreviewServerRegistryStore {
  private records = new Map<string, PreviewServerRecord>()

  save(record: PreviewServerRecord): Promise<void> {
    this.records.set(record.serverId, { ...record })
    return Promise.resolve()
  }

  get(serverId: string): Promise<PreviewServerRecord | null> {
    const record = this.records.get(serverId)
    return Promise.resolve(record ? { ...record } : null)
  }

  delete(serverId: string): Promise<void> {
    this.records.delete(serverId)
    return Promise.resolve()
  }

  list(): Promise<PreviewServerRecord[]> {
    return Promise.resolve(Array.from(this.records.values()).map((record) => ({ ...record })))
  }
}

export class PreviewServerRegistry implements PreviewServerLifecycleSink {
  private readonly cache = new Map<string, PreviewServerRecord>()

  constructor(
    private readonly store: PreviewServerRegistryStore = new InMemoryPreviewServerRegistryStore(),
    private readonly logger?: Logger,
  ) {}

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

  async register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void> {
    const existing = this.cache.get(serverId) ?? (await this.store.get(serverId)) ?? undefined
    const now = new Date()

    const record: PreviewServerRecord = {
      serverId,
      status: snapshot.status ?? existing?.status ?? ('stopped' as ServerStatusType),
      inspectorEnabled: snapshot.inspectorEnabled ?? existing?.inspectorEnabled ?? true,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
    }

    this.assignOptionalFields(record, snapshot)

    await this.store.save(record)
    this.cache.set(serverId, record)
  }

  async update(serverId: string, updates: PreviewServerUpdate): Promise<void> {
    const existing = this.cache.get(serverId) ?? (await this.store.get(serverId)) ?? undefined

    if (!existing) {
      this.logger?.warn('PreviewServerRegistry: Cannot update missing record', { serverId })
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

    await this.store.save(record)
    this.cache.set(serverId, record)
  }

  async remove(serverId: string): Promise<void> {
    this.cache.delete(serverId)
    await this.store.delete(serverId)
  }

  async getRecord(serverId: string): Promise<PreviewServerRecord | null> {
    const cached = this.cache.get(serverId)
    if (cached) {
      return cached
    }

    const record = await this.store.get(serverId)
    if (record) {
      this.cache.set(serverId, record)
    }
    return record
  }

  async listRecords(): Promise<PreviewServerRecord[]> {
    const records = await this.store.list()
    for (const record of records) {
      this.cache.set(record.serverId, record)
    }
    return records
  }
}
