import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database.js'
import { DatabaseSymbol } from '../../../infrastructure/database/database.js'
import { logger } from '../../../utils/logger.js'
import type { PreviewServerRecord } from '../types/registry.js'
import { previewServers, recordToInsert, selectToRecord } from './preview-server.schema.js'

/**
 * Repository for preview server record persistence using Drizzle ORM.
 * Provides type-safe database operations for preview server lifecycle state.
 *
 * This replaces the legacy PreviewRegistryStorage adapter, using a relational database
 * instead of JSON file storage.
 */
@injectable()
export class PreviewServerRepository {
  private readonly serviceLogger: typeof logger

  constructor(@inject(DatabaseSymbol) private readonly db: Database) {
    this.serviceLogger = logger.child({ service: 'preview-server-repository' })
  }

  /**
   * Save a preview server record to the database.
   * Creates a new record or updates an existing one (upsert).
   *
   * @param record - Preview server record to save
   */
  async save(record: PreviewServerRecord): Promise<void> {
    try {
      const insert = recordToInsert(record)

      await this.db.drizzle
        .insert(previewServers)
        .values(insert)
        .onConflictDoUpdate({
          target: previewServers.serverId,
          set: {
            status: insert.status,
            port: insert.port,
            url: insert.url,
            proxyUrl: insert.proxyUrl,
            hostname: insert.hostname,
            pid: insert.pid,
            inspectorEnabled: insert.inspectorEnabled,
            error: insert.error,
            metadata: insert.metadata,
            updatedAt: insert.updatedAt,
          },
        })

      this.serviceLogger.debug('Preview server record saved', { serverId: record.serverId })
    } catch (error) {
      this.serviceLogger.error('Failed to save preview server record', error as Error, {
        serverId: record.serverId,
      })
      throw error
    }
  }

  /**
   * Get a preview server record by ID.
   *
   * @param serverId - Server identifier
   * @returns Preview server record or null if not found
   */
  async get(serverId: string): Promise<PreviewServerRecord | null> {
    try {
      const rows = await this.db.drizzle
        .select()
        .from(previewServers)
        .where(eq(previewServers.serverId, serverId))
        .limit(1)

      const row = rows[0]
      if (!row) {
        return null
      }

      return selectToRecord(row)
    } catch (error) {
      this.serviceLogger.error('Failed to read preview server record', error as Error, {
        serverId,
      })
      throw error
    }
  }

  /**
   * Delete a preview server record by ID.
   *
   * @param serverId - Server identifier
   */
  async delete(serverId: string): Promise<void> {
    try {
      await this.db.drizzle.delete(previewServers).where(eq(previewServers.serverId, serverId))
      this.serviceLogger.debug('Preview server record deleted', { serverId })
    } catch (error) {
      this.serviceLogger.error('Failed to delete preview server record', error as Error, {
        serverId,
      })
      // Don't throw if already deleted - make delete idempotent
    }
  }

  /**
   * List all preview server records.
   * Use with caution - consider pagination for production use.
   *
   * @returns Array of all preview server records
   */
  async list(): Promise<PreviewServerRecord[]> {
    try {
      const rows = await this.db.drizzle.select().from(previewServers)
      return rows.map(selectToRecord)
    } catch (error) {
      this.serviceLogger.error('Failed to list preview server records', error as Error)
      return [] // Return empty array on error (matches legacy storage behavior)
    }
  }
}
