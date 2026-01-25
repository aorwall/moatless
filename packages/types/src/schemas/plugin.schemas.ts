import { z } from 'zod'

/**
 * Plugin owner information in marketplace manifest.
 */
export const PluginOwnerSchema = z.object({
  name: z.string(),
  email: z.string().optional(),
})

/**
 * Plugin metadata in marketplace manifest.
 */
export const MarketplaceMetadataSchema = z.object({
  description: z.string().optional(),
  version: z.string().optional(),
  pluginRoot: z.string().optional(),
})

/**
 * Individual plugin entry in marketplace manifest.
 */
export const MarketplacePluginEntrySchema = z.object({
  name: z.string(),
  source: z.string(), // relative path like "./plugins/claude/context7"
  description: z.string().optional(),
  version: z.string().optional(),
})

/**
 * Marketplace manifest from .claude-plugin/marketplace.json.
 * Defines the structure of a plugin marketplace repository.
 */
export const MarketplaceManifestSchema = z.object({
  name: z.string(),
  owner: PluginOwnerSchema,
  metadata: MarketplaceMetadataSchema.optional(),
  plugins: z.array(MarketplacePluginEntrySchema),
})

/**
 * Plugin marketplace stored in database.
 */
export const PluginMarketplaceSchema = z.object({
  id: z.string(),
  name: z.string(),
  repoUrl: z.string(),
  ownerName: z.string().optional(),
  description: z.string().optional(),
  lastSyncedAt: z.coerce.date().optional(),
  createdAt: z.coerce.date(),
  updatedAt: z.coerce.date(),
})

/**
 * Plugin info returned by GET /plugins API.
 * Contains information about a plugin from a marketplace.
 */
export const PluginInfoSchema = z.object({
  id: z.string(), // format: {plugin-name}@{marketplace-name}
  name: z.string(),
  marketplaceName: z.string(),
  marketplaceId: z.string(),
  description: z.string().optional(),
  version: z.string().optional(),
  source: z.string(), // relative path in marketplace
})

// ============================================================================
// API Request/Response Schemas
// ============================================================================

/**
 * Request to add a new marketplace.
 */
export const AddMarketplaceRequestSchema = z.object({
  repoUrl: z.string().url('Must be a valid Git URL'),
})

/**
 * Response for listing plugins.
 */
export const PluginListResponseSchema = z.object({
  plugins: z.array(PluginInfoSchema),
})

/**
 * Response for listing marketplaces.
 */
export const MarketplaceListResponseSchema = z.object({
  marketplaces: z.array(PluginMarketplaceSchema),
})

/**
 * Plugin marketplace ID path parameter.
 */
export const MarketplaceIdParamsSchema = z.object({
  marketplaceId: z.string().describe('Plugin marketplace identifier'),
})

// ============================================================================
// Type Exports
// ============================================================================

export type PluginOwner = z.infer<typeof PluginOwnerSchema>
export type MarketplaceMetadata = z.infer<typeof MarketplaceMetadataSchema>
export type MarketplacePluginEntry = z.infer<typeof MarketplacePluginEntrySchema>
export type MarketplaceManifest = z.infer<typeof MarketplaceManifestSchema>
export type PluginMarketplace = z.infer<typeof PluginMarketplaceSchema>
export type PluginInfo = z.infer<typeof PluginInfoSchema>
export type AddMarketplaceRequest = z.infer<typeof AddMarketplaceRequestSchema>
export type PluginListResponse = z.infer<typeof PluginListResponseSchema>
export type MarketplaceListResponse = z.infer<typeof MarketplaceListResponseSchema>
