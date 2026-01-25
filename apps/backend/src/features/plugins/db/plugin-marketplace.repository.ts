import type { PluginMarketplace } from '@moatless/types'
import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import {
  marketplaceToInsert,
  pluginMarketplaces,
  selectToMarketplace,
} from './plugin-marketplace.schema'

/**
 * Repository for plugin marketplace persistence using Drizzle ORM.
 * Provides type-safe database operations for plugin marketplace entities.
 */
@injectable()
export class PluginMarketplaceRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a plugin marketplace to the database.
   * Creates a new marketplace or updates an existing one (upsert).
   *
   * @param marketplace - Plugin marketplace configuration to save
   */
  async save(marketplace: PluginMarketplace): Promise<void> {
    const insert = marketplaceToInsert(marketplace)

    await this.db.drizzle
      .insert(pluginMarketplaces)
      .values(insert)
      .onConflictDoUpdate({
        target: pluginMarketplaces.id,
        set: {
          name: insert.name,
          repoUrl: insert.repoUrl,
          ownerName: insert.ownerName,
          description: insert.description,
          lastSyncedAt: insert.lastSyncedAt,
          updatedAt: insert.updatedAt,
        },
      })
  }

  /**
   * Get a plugin marketplace by ID.
   *
   * @param id - Marketplace identifier
   * @returns Plugin marketplace configuration or null if not found
   */
  async get(id: string): Promise<PluginMarketplace | null> {
    const rows = await this.db.drizzle
      .select()
      .from(pluginMarketplaces)
      .where(eq(pluginMarketplaces.id, id))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToMarketplace(row)
  }

  /**
   * Get a plugin marketplace by name.
   *
   * @param name - Marketplace name
   * @returns Plugin marketplace configuration or null if not found
   */
  async getByName(name: string): Promise<PluginMarketplace | null> {
    const rows = await this.db.drizzle
      .select()
      .from(pluginMarketplaces)
      .where(eq(pluginMarketplaces.name, name))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToMarketplace(row)
  }

  /**
   * Get all plugin marketplaces.
   *
   * @returns Array of all plugin marketplace configurations
   */
  async getAll(): Promise<PluginMarketplace[]> {
    const rows = await this.db.drizzle.select().from(pluginMarketplaces)

    return rows.map(selectToMarketplace)
  }

  /**
   * Delete a plugin marketplace by ID.
   *
   * @param id - Marketplace identifier
   */
  async delete(id: string): Promise<void> {
    await this.db.drizzle.delete(pluginMarketplaces).where(eq(pluginMarketplaces.id, id))
  }

  /**
   * Check if a plugin marketplace exists.
   *
   * @param id - Marketplace identifier
   * @returns True if marketplace exists
   */
  async exists(id: string): Promise<boolean> {
    const rows = await this.db.drizzle
      .select({ id: pluginMarketplaces.id })
      .from(pluginMarketplaces)
      .where(eq(pluginMarketplaces.id, id))
      .limit(1)

    return rows.length > 0
  }

  /**
   * Check if a plugin marketplace exists by name.
   *
   * @param name - Marketplace name
   * @returns True if marketplace exists
   */
  async existsByName(name: string): Promise<boolean> {
    const rows = await this.db.drizzle
      .select({ id: pluginMarketplaces.id })
      .from(pluginMarketplaces)
      .where(eq(pluginMarketplaces.name, name))
      .limit(1)

    return rows.length > 0
  }
}
