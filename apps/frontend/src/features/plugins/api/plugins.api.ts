import type {
  AddMarketplaceRequest,
  MarketplaceListResponse,
  PluginListResponse,
  PluginMarketplace,
} from '@/lib/types/plugins'

// TODO: Plugin/marketplace endpoints not implemented in Rust backend
export const pluginsApi = {
  listPlugins: (): Promise<PluginListResponse> => {
    return Promise.resolve({ plugins: [] } as PluginListResponse)
  },

  listMarketplaces: (): Promise<MarketplaceListResponse> => {
    return Promise.resolve({ marketplaces: [] } as MarketplaceListResponse)
  },

  addMarketplace: (_request: AddMarketplaceRequest): Promise<PluginMarketplace> => {
    return Promise.resolve({} as PluginMarketplace)
  },

  removeMarketplace: (_marketplaceId: string): Promise<void> => {
    return Promise.resolve()
  },

  syncMarketplace: (_marketplaceId: string): Promise<PluginMarketplace> => {
    return Promise.resolve({} as PluginMarketplace)
  },
}
