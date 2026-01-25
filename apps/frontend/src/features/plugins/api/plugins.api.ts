import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  addMarketplaceRoute,
  deleteMarketplaceRoute,
  listMarketplacesRoute,
  listPluginsRoute,
  syncMarketplaceRoute,
} from '@moatless/api-contract/plugins'
import { apiClient } from '@/lib/api/client'

type ListPluginsResponse = RouteSuccess<typeof listPluginsRoute>
type ListMarketplacesResponse = RouteSuccess<typeof listMarketplacesRoute>
type AddMarketplaceRequest = RouteRequestJson<typeof addMarketplaceRoute>
type AddMarketplaceResponse = RouteSuccess<typeof addMarketplaceRoute>
type SyncMarketplaceResponse = RouteSuccess<typeof syncMarketplaceRoute>

export const pluginsApi = {
  /**
   * List all available plugins from all marketplaces
   */
  listPlugins: async (): Promise<ListPluginsResponse> => {
    const path = buildPath(listPluginsRoute)
    return await apiClient.get<ListPluginsResponse>(path)
  },

  /**
   * List all plugin marketplaces
   */
  listMarketplaces: async (): Promise<ListMarketplacesResponse> => {
    const path = buildPath(listMarketplacesRoute)
    return await apiClient.get<ListMarketplacesResponse>(path)
  },

  /**
   * Add a new plugin marketplace by cloning from a GitHub repository
   */
  addMarketplace: async (request: AddMarketplaceRequest): Promise<AddMarketplaceResponse> => {
    const path = buildPath(addMarketplaceRoute)
    return await apiClient.post<AddMarketplaceResponse>(path, request)
  },

  /**
   * Remove a plugin marketplace
   */
  removeMarketplace: async (marketplaceId: string): Promise<void> => {
    const path = buildPath(deleteMarketplaceRoute, { marketplaceId })
    return await apiClient.delete<void>(path)
  },

  /**
   * Sync a marketplace from remote
   */
  syncMarketplace: async (marketplaceId: string): Promise<SyncMarketplaceResponse> => {
    const path = buildPath(syncMarketplaceRoute, { marketplaceId })
    return await apiClient.post<SyncMarketplaceResponse>(path)
  },
}
