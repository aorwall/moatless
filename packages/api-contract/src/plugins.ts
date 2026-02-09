import { createRoute } from '@hono/zod-openapi'
import {
  AddMarketplaceRequestSchema,
  MarketplaceIdParamsSchema,
  MarketplaceListResponseSchema,
  PluginIdParamsSchema,
  PluginListResponseSchema,
  PluginMarketplaceSchema,
  PluginVersionResponseSchema,
} from '@moatless/types'

// ============================================================================
// Plugin Routes
// ============================================================================

/**
 * GET /api/v1/plugins
 * List all available plugins from all marketplaces
 */
export const listPluginsRoute = createRoute({
  method: 'get',
  path: '/api/v1/plugins',
  tags: ['Plugins'],
  description: 'List all available plugins from all marketplaces',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: PluginListResponseSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/plugins/marketplaces
 * List all plugin marketplaces
 */
export const listMarketplacesRoute = createRoute({
  method: 'get',
  path: '/api/v1/plugins/marketplaces',
  tags: ['Plugins'],
  description: 'List all plugin marketplaces',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: MarketplaceListResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/plugins/marketplaces
 * Add a new plugin marketplace by cloning from a GitHub repository
 */
export const addMarketplaceRoute = createRoute({
  method: 'post',
  path: '/api/v1/plugins/marketplaces',
  tags: ['Plugins'],
  description: 'Add a new plugin marketplace by cloning from a public GitHub repository',
  request: {
    body: {
      content: {
        'application/json': {
          schema: AddMarketplaceRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Marketplace added successfully',
      content: {
        'application/json': {
          schema: PluginMarketplaceSchema,
        },
      },
    },
    400: {
      description: 'Invalid request or repository URL',
    },
    500: {
      description: 'Failed to clone or parse marketplace',
    },
  },
})

/**
 * DELETE /api/v1/plugins/marketplaces/{marketplaceId}
 * Remove a plugin marketplace
 */
export const deleteMarketplaceRoute = createRoute({
  method: 'delete',
  path: '/api/v1/plugins/marketplaces/{marketplaceId}',
  tags: ['Plugins'],
  description: 'Remove a plugin marketplace and delete its local files',
  request: {
    params: MarketplaceIdParamsSchema,
  },
  responses: {
    204: {
      description: 'Marketplace removed successfully',
    },
    404: {
      description: 'Marketplace not found',
    },
  },
})

/**
 * POST /api/v1/plugins/marketplaces/{marketplaceId}/sync
 * Sync a marketplace from remote
 */
export const syncMarketplaceRoute = createRoute({
  method: 'post',
  path: '/api/v1/plugins/marketplaces/{marketplaceId}/sync',
  tags: ['Plugins'],
  description: 'Sync a marketplace by pulling latest changes from remote repository',
  request: {
    params: MarketplaceIdParamsSchema,
  },
  responses: {
    200: {
      description: 'Marketplace synced successfully',
      content: {
        'application/json': {
          schema: PluginMarketplaceSchema,
        },
      },
    },
    404: {
      description: 'Marketplace not found',
    },
    500: {
      description: 'Failed to sync marketplace',
    },
  },
})

// ============================================================================
// Plugin Download Routes (for sandbox API-based fetching)
// ============================================================================

/**
 * GET /api/v1/plugins/{pluginId}/download
 * Download a plugin as a TAR archive for sandbox use.
 * Authenticated via X-User-Token header.
 */
export const downloadPluginRoute = createRoute({
  method: 'get',
  path: '/api/v1/plugins/{pluginId}/download',
  tags: ['Plugins'],
  description: 'Download a plugin as a TAR archive. Used by sandbox at startup.',
  request: {
    params: PluginIdParamsSchema,
  },
  responses: {
    200: {
      description: 'Plugin TAR archive',
      content: {
        'application/x-tar': {
          schema: { type: 'string', format: 'binary' },
        },
      },
    },
    404: {
      description: 'Plugin not found',
    },
    500: {
      description: 'Failed to create plugin archive',
    },
  },
})

/**
 * GET /api/v1/plugins/{pluginId}/version
 * Get plugin version for cache validation.
 * Authenticated via X-User-Token header.
 */
export const getPluginVersionRoute = createRoute({
  method: 'get',
  path: '/api/v1/plugins/{pluginId}/version',
  tags: ['Plugins'],
  description: 'Get plugin version for cache validation. Used by sandbox before downloading.',
  request: {
    params: PluginIdParamsSchema,
  },
  responses: {
    200: {
      description: 'Plugin version information',
      content: {
        'application/json': {
          schema: PluginVersionResponseSchema,
        },
      },
    },
    404: {
      description: 'Plugin not found',
    },
  },
})
