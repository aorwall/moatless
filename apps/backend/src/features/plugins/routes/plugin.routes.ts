import { Readable } from 'node:stream'
import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  addMarketplaceRoute,
  deleteMarketplaceRoute,
  downloadPluginRoute,
  getPluginVersionRoute,
  listMarketplacesRoute,
  listPluginsRoute,
  syncMarketplaceRoute,
} from '@moatless/api-contract/plugins'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { logger } from '../../../utils/logger'
import { PluginService } from '../services/plugin.service'

/**
 * Register plugin routes for managing plugin marketplaces.
 */
export function registerPluginRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * GET /api/v1/plugins
   * List all available plugins from all marketplaces
   */
  app.openapi(listPluginsRoute, async (c) => {
    try {
      const pluginService = appContainer.get(PluginService)
      const plugins = await pluginService.listPlugins()
      return c.json({ plugins })
    } catch (error) {
      logger.error({ err: error }, 'Failed to list plugins')
      throw new HTTPException(500, {
        message: error instanceof Error ? error.message : 'Failed to list plugins',
      })
    }
  })

  /**
   * GET /api/v1/plugins/marketplaces
   * List all plugin marketplaces
   */
  app.openapi(listMarketplacesRoute, async (c) => {
    try {
      const pluginService = appContainer.get(PluginService)
      const marketplaces = await pluginService.listMarketplaces()
      return c.json({ marketplaces })
    } catch (error) {
      logger.error({ err: error }, 'Failed to list marketplaces')
      throw new HTTPException(500, {
        message: error instanceof Error ? error.message : 'Failed to list marketplaces',
      })
    }
  })

  /**
   * POST /api/v1/plugins/marketplaces
   * Add a new plugin marketplace
   */
  app.openapi(addMarketplaceRoute, async (c) => {
    try {
      const body = c.req.valid('json')
      const pluginService = appContainer.get(PluginService)
      const marketplace = await pluginService.addMarketplace(body.repoUrl)
      return c.json(marketplace)
    } catch (error) {
      logger.error({ err: error }, 'Failed to add marketplace')
      if (
        error instanceof Error &&
        (error.message.includes('not found') || error.message.includes('Invalid'))
      ) {
        throw new HTTPException(400, { message: error.message })
      }
      throw new HTTPException(500, {
        message: error instanceof Error ? error.message : 'Failed to add marketplace',
      })
    }
  })

  /**
   * DELETE /api/v1/plugins/marketplaces/{marketplaceId}
   * Remove a plugin marketplace
   */
  app.openapi(deleteMarketplaceRoute, async (c) => {
    try {
      const { marketplaceId } = c.req.valid('param')
      const pluginService = appContainer.get(PluginService)
      await pluginService.removeMarketplace(marketplaceId)
      return c.body(null, 204)
    } catch (error) {
      logger.error({ err: error }, 'Failed to remove marketplace')
      if (error instanceof Error && error.message.includes('not found')) {
        throw new HTTPException(404, { message: error.message })
      }
      throw new HTTPException(500, {
        message: error instanceof Error ? error.message : 'Failed to remove marketplace',
      })
    }
  })

  /**
   * POST /api/v1/plugins/marketplaces/{marketplaceId}/sync
   * Sync a marketplace from remote
   */
  app.openapi(syncMarketplaceRoute, async (c) => {
    try {
      const { marketplaceId } = c.req.valid('param')
      const pluginService = appContainer.get(PluginService)
      const marketplace = await pluginService.syncMarketplace(marketplaceId)
      return c.json(marketplace)
    } catch (error) {
      logger.error({ err: error }, 'Failed to sync marketplace')
      if (error instanceof Error && error.message.includes('not found')) {
        throw new HTTPException(404, { message: error.message })
      }
      throw new HTTPException(500, {
        message: error instanceof Error ? error.message : 'Failed to sync marketplace',
      })
    }
  })

  /**
   * GET /api/v1/plugins/{pluginId}/download
   * Download a plugin as a TAR archive for sandbox use.
   * Authenticated via X-User-Token header.
   */
  app.openapi(downloadPluginRoute, async (c) => {
    try {
      const { pluginId } = c.req.valid('param')
      const pluginService = appContainer.get(PluginService)

      const archiveStream = await pluginService.streamPluginArchive(pluginId)
      if (!archiveStream) {
        throw new HTTPException(404, { message: `Plugin not found: ${pluginId}` })
      }

      // Convert Node.js Readable to Web ReadableStream for Hono response
      const webStream = Readable.toWeb(archiveStream) as ReadableStream<Uint8Array>

      return new Response(webStream, {
        headers: {
          'Content-Type': 'application/x-tar',
          'Content-Disposition': `attachment; filename="${pluginId.replace('@', '-')}.tar"`,
        },
      })
    } catch (error) {
      logger.error({ err: error }, 'Failed to download plugin')
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, {
        message: error instanceof Error ? error.message : 'Failed to download plugin',
      })
    }
  })

  /**
   * GET /api/v1/plugins/{pluginId}/version
   * Get plugin version for cache validation.
   */
  app.openapi(getPluginVersionRoute, async (c) => {
    try {
      const { pluginId } = c.req.valid('param')
      const pluginService = appContainer.get(PluginService)

      const versionInfo = await pluginService.getPluginVersion(pluginId)
      if (!versionInfo) {
        throw new HTTPException(404, { message: `Plugin not found: ${pluginId}` })
      }

      return c.json(versionInfo)
    } catch (error) {
      logger.error({ err: error }, 'Failed to get plugin version')
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, {
        message: error instanceof Error ? error.message : 'Failed to get plugin version',
      })
    }
  })
}
