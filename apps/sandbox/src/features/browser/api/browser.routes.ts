/**
 * Browser API Routes - Minimal endpoints for browser lifecycle and screenshot serving
 *
 * HTTP API for browser automation:
 * - GET /browser - Server info + wsEndpoint for CDP connections
 * - POST /browser/start - Start browser, return wsEndpoint
 * - POST /browser/stop - Stop browser
 * - GET /browser/screenshots - List screenshots from disk
 * - GET /browser/screenshots/:filename - Serve screenshot image
 * - DELETE /browser/screenshots - Clear all screenshots
 *
 * All page management and actions are handled client-side via CDP.
 * Scripts connect via wsEndpoint and use Playwright directly.
 */

import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'
import { logger as rootLogger } from '../../../shared/logger'
import { getBrowserServer } from '../services/browser-server'

const logger = rootLogger.child({ service: 'browser-routes' })

/**
 * Register browser routes
 */
export function registerBrowserRoutes(app: Hono<AppBindings>) {
  // GET /browser - Get browser server info
  app.get('/browser', (c) => {
    const server = getBrowserServer()
    const info = server.getInfo()
    return c.json(info)
  })

  // POST /browser/start - Start browser server
  app.post('/browser/start', async (c) => {
    try {
      const server = getBrowserServer()

      if (server.isRunning()) {
        return c.json({
          success: true,
          message: 'Browser server already running',
          ...server.getInfo(),
        })
      }

      const { wsEndpoint } = await server.start()

      return c.json({
        success: true,
        message: 'Browser server started',
        ...server.getInfo(),
        wsEndpoint,
      })
    } catch (error) {
      logger.error({ err: error }, 'Failed to start browser server')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // POST /browser/stop - Stop browser server
  app.post('/browser/stop', async (c) => {
    try {
      const server = getBrowserServer()

      if (!server.isRunning()) {
        return c.json({
          success: true,
          message: 'Browser server not running',
        })
      }

      await server.stop()

      return c.json({
        success: true,
        message: 'Browser server stopped',
      })
    } catch (error) {
      logger.error({ err: error }, 'Failed to stop browser server')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // ==================== Screenshot Routes ====================

  // GET /browser/screenshots - List all screenshots from disk
  app.get('/browser/screenshots', async (c) => {
    const server = getBrowserServer()
    const screenshots = await server.listScreenshots()
    return c.json({
      screenshots,
      total: screenshots.length,
    })
  })

  // GET /browser/screenshots/:filename - Serve screenshot image
  app.get('/browser/screenshots/:filename', async (c) => {
    const server = getBrowserServer()
    const filename = c.req.param('filename')

    // Validate filename to prevent directory traversal
    if (filename.includes('/') || filename.includes('\\') || filename.includes('..')) {
      return c.json({ error: 'Invalid filename' }, 400)
    }

    const exists = await server.screenshotExists(filename)
    if (!exists) {
      return c.json({ error: 'Screenshot not found' }, 404)
    }

    try {
      const buffer = await server.readScreenshot(filename)
      // Convert Buffer to Uint8Array for Response compatibility
      const uint8Array = new Uint8Array(buffer)
      return new Response(uint8Array, {
        headers: {
          'Content-Type': 'image/png',
          'Content-Length': buffer.length.toString(),
          'Cache-Control': 'public, max-age=31536000, immutable',
        },
      })
    } catch (error) {
      logger.error({ err: error, filename }, 'Failed to read screenshot')
      return c.json({ error: 'Failed to read screenshot' }, 500)
    }
  })

  // DELETE /browser/screenshots - Clear all screenshots
  app.delete('/browser/screenshots', async (c) => {
    const server = getBrowserServer()

    try {
      const count = await server.clearScreenshots()
      return c.json({
        success: true,
        message: `Cleared ${count} screenshots`,
        deleted: count,
      })
    } catch (error) {
      logger.error({ err: error }, 'Failed to clear screenshots')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })
}
