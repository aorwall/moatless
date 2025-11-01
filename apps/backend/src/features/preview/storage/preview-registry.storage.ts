import path from 'node:path'
import { inject, injectable } from 'inversify'
import { type StorageBackend, StorageSymbol } from '../../../storage/storage-backend.js'
import { logger } from '../../../utils/logger.js'
import type { PreviewServerRecord } from '../types/index.js'

/**
 * PreviewRegistryStorage - Persists preview server records using StorageBackend
 *
 * Storage schema:
 * - Base path: ${dataDir}/preview-registry/
 * - Per-record: ${serverId}.json
 *
 * This storage adapter enables preview server registry to survive restarts.
 */
@injectable()
export class PreviewRegistryStorage {
  private readonly serviceLogger: typeof logger
  private readonly basePath = 'preview-registry'

  constructor(@inject(StorageSymbol) private readonly storage: StorageBackend) {
    this.serviceLogger = logger.child({ service: 'preview-registry-storage' })
  }

  /**
   * Save a preview server record
   */
  async save(record: PreviewServerRecord): Promise<void> {
    try {
      const key = this.getRecordPath(record.serverId)
      await this.storage.save(key, record)
      this.serviceLogger.debug('Preview record saved', { serverId: record.serverId })
    } catch (error) {
      this.serviceLogger.error('Failed to save preview record', error as Error, {
        serverId: record.serverId,
      })
      throw error
    }
  }

  /**
   * Get a preview server record by ID
   */
  async get(serverId: string): Promise<PreviewServerRecord | null> {
    try {
      const key = this.getRecordPath(serverId)
      const record = await this.storage.get<PreviewServerRecord>(key)

      if (!record) {
        return null
      }

      // Deserialize dates
      record.createdAt = new Date(record.createdAt)
      record.updatedAt = new Date(record.updatedAt)

      return record
    } catch (error) {
      this.serviceLogger.error('Failed to read preview record', error as Error, { serverId })
      throw error
    }
  }

  /**
   * Delete a preview server record
   */
  async delete(serverId: string): Promise<void> {
    try {
      const key = this.getRecordPath(serverId)
      await this.storage.delete(key)
      this.serviceLogger.debug('Preview record deleted', { serverId })
    } catch (error) {
      this.serviceLogger.error('Failed to delete preview record', error as Error, { serverId })
      // Don't throw if already deleted
    }
  }

  /**
   * List all preview server records
   */
  async list(): Promise<PreviewServerRecord[]> {
    try {
      const keys = await this.storage.list(this.basePath)
      const records: PreviewServerRecord[] = []

      for (const key of keys) {
        if (key.endsWith('.json')) {
          const serverId = path.basename(key, '.json')
          const record = await this.get(serverId)
          if (record) {
            records.push(record)
          }
        }
      }

      return records
    } catch (error) {
      this.serviceLogger.error('Failed to list preview records', error as Error)
      return [] // Return empty array on error
    }
  }

  /**
   * Get file path for a server record
   */
  private getRecordPath(serverId: string): string {
    return path.join(this.basePath, `${serverId}.json`)
  }
}
