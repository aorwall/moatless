/**
 * Plugins feature module.
 * Provides plugin management for Claude Code plugins from marketplaces.
 */

// Export API client
export { pluginsApi } from './api/plugins.api'

// Export components
export { MarketplacesPage } from './components/marketplaces-page'

// Export hooks
export {
  useAddMarketplace,
  useMarketplaces,
  usePlugins,
  useRemoveMarketplace,
  useSyncMarketplace,
} from './hooks/use-plugins'
