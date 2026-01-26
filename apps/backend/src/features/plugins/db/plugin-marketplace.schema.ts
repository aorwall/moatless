import type { PluginMarketplace } from '@moatless/types'
import { index, pgTable, text, timestamp } from 'drizzle-orm/pg-core'

/**
 * Plugin marketplaces table schema (PostgreSQL).
 * Stores plugin marketplace configurations for downloading plugins from GitHub repos.
 */
export const pluginMarketplaces = pgTable(
  'plugin_marketplaces',
  {
    // Primary key - marketplace UUID
    id: text('id').primaryKey(),

    // Marketplace name (must be unique)
    name: text('name').notNull().unique(),

    // GitHub repository URL
    repoUrl: text('repo_url').notNull(),

    // Owner name from marketplace.json
    ownerName: text('owner_name'),

    // Description from marketplace.json metadata
    description: text('description'),

    // Sync status: 'synced' | 'failed' | 'syncing' | 'unknown'
    status: text('status').notNull().default('unknown'),

    // Last error message if status is 'failed'
    lastError: text('last_error'),

    // Last time marketplace was synced from remote
    lastSyncedAt: timestamp('last_synced_at'),

    // Timestamps
    createdAt: timestamp('created_at').notNull(),
    updatedAt: timestamp('updated_at').notNull(),
  },
  (table) => ({
    // Index for name lookups
    nameIdx: index('plugin_marketplaces_name_idx').on(table.name),
  }),
)

/**
 * Inferred TypeScript type for inserting a plugin marketplace.
 */
export type PluginMarketplaceInsert = typeof pluginMarketplaces.$inferInsert

/**
 * Inferred TypeScript type for selecting a plugin marketplace from the database.
 */
export type PluginMarketplaceSelect = typeof pluginMarketplaces.$inferSelect

/**
 * Convert PluginMarketplace domain type to database insert type.
 */
export function marketplaceToInsert(marketplace: PluginMarketplace): PluginMarketplaceInsert {
  return {
    id: marketplace.id,
    name: marketplace.name,
    repoUrl: marketplace.repoUrl,
    ownerName: marketplace.ownerName ?? null,
    description: marketplace.description ?? null,
    status: marketplace.status ?? 'unknown',
    lastError: marketplace.lastError ?? null,
    lastSyncedAt: marketplace.lastSyncedAt ?? null,
    createdAt: marketplace.createdAt ?? new Date(),
    updatedAt: marketplace.updatedAt ?? new Date(),
  }
}

/**
 * Convert database row to PluginMarketplace domain type.
 */
export function selectToMarketplace(row: PluginMarketplaceSelect): PluginMarketplace {
  return {
    id: row.id,
    name: row.name,
    repoUrl: row.repoUrl,
    ownerName: row.ownerName ?? undefined,
    description: row.description ?? undefined,
    status: (row.status as 'synced' | 'failed' | 'syncing' | 'unknown') ?? 'unknown',
    lastError: row.lastError ?? undefined,
    lastSyncedAt: row.lastSyncedAt ? new Date(row.lastSyncedAt) : undefined,
    createdAt: new Date(row.createdAt),
    updatedAt: new Date(row.updatedAt),
  }
}
