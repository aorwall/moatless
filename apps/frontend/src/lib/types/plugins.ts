/**
 * Plugin and marketplace types for endpoints not yet implemented in the Rust backend.
 * When the backend adds these endpoints, replace with api-client generated types.
 */

export interface PluginInfo {
  id: string
  name: string
  marketplaceName: string
  marketplaceId: string
  description?: string
  version?: string
  source: string
}

export interface PluginMarketplace {
  id: string
  name: string
  repoUrl: string
  ownerName?: string
  description?: string
  status: 'synced' | 'failed' | 'syncing' | 'unknown'
  lastError?: string
  lastSyncedAt?: Date
  createdAt: Date
  updatedAt: Date
}

export interface AddMarketplaceRequest {
  repoUrl: string
}

export interface PluginListResponse {
  plugins: PluginInfo[]
}

export interface MarketplaceListResponse {
  marketplaces: PluginMarketplace[]
}
