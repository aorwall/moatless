import { createRoute } from '@hono/zod-openapi'
import {
  AddMarketplaceRequestSchema,
  MarketplaceIdParamsSchema,
  MarketplaceListResponseSchema,
  PluginListResponseSchema,
  PluginMarketplaceSchema,
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
