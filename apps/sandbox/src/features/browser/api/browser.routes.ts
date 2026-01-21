/**
 * Browser API Routes - Minimal endpoints for browser lifecycle and screenshot serving
 *
 * HTTP API for browser automation:
 * - GET /browser - Server info + wsEndpoint for CDP connections
 * - POST /browser/start - Start browser, return wsEndpoint
 * - POST /browser/stop - Stop browser
 * - POST /browser/screenshot - Capture screenshot of preview server
 * - GET /browser/screenshots - List screenshots from disk
 * - GET /browser/screenshots/:filename - Serve screenshot image
 * - DELETE /browser/screenshots - Clear all screenshots
 *
 * All page management and actions are handled client-side via CDP.
 * Scripts connect via wsEndpoint and use native CDP.
 */

import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'
import { logger as rootLogger } from '../../../shared/logger'
import { getBrowserServer } from '../services/browser-server'

const logger = rootLogger.child({ service: 'browser-routes' })

// ============================================================================
// Native CDP Client for Screenshots (using Bun's WebSocket)
// ============================================================================

interface CDPRequest {
  id: number
  method: string
  params?: Record<string, unknown>
  sessionId?: string
}

interface CDPResponse {
  id?: number
  result?: Record<string, unknown>
  error?: { code: number; message: string }
  method?: string
  params?: Record<string, unknown>
  sessionId?: string
}

/**
 * Simple CDP client for screenshot operations
 */
class ScreenshotCDPClient {
  private ws: WebSocket | undefined
  private requestId = 0
  private pendingRequests = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (reason: Error) => void }
  >()
  private connected = false
  private sessionId: string | undefined
  private targetId: string | undefined

  async connect(cdpPort: number): Promise<void> {
    // Fetch WebSocket URL from browser's JSON endpoint
    const response = await fetch(`http://localhost:${cdpPort}/json/version`)
    if (!response.ok) {
      throw new Error(`Failed to get browser version info: ${response.statusText}`)
    }
    const versionInfo = (await response.json()) as { webSocketDebuggerUrl: string }
    const wsUrl = versionInfo.webSocketDebuggerUrl

    return new Promise((resolve, reject) => {
      this.ws = new WebSocket(wsUrl)

      this.ws.onopen = () => {
        this.connected = true
        resolve()
      }

      this.ws.onerror = (event) => {
        reject(new Error(`WebSocket connection error: ${event}`))
      }

      this.ws.onclose = () => {
        this.connected = false
        for (const [, { reject: rejectPending }] of this.pendingRequests) {
          rejectPending(new Error('WebSocket connection closed'))
        }
        this.pendingRequests.clear()
      }

      this.ws.onmessage = (event) => {
        const message: CDPResponse = JSON.parse(event.data as string)
        if (message.id !== undefined) {
          const pending = this.pendingRequests.get(message.id)
          if (pending) {
            this.pendingRequests.delete(message.id)
            if (message.error) {
              pending.reject(new Error(`CDP Error: ${message.error.message}`))
            } else {
              pending.resolve(message.result)
            }
          }
        }
      }
    })
  }

  send<T = Record<string, unknown>>(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<T> {
    if (!(this.ws && this.connected)) {
      return Promise.reject(new Error('CDP client not connected'))
    }

    const id = ++this.requestId
    const request: CDPRequest = { id, method }
    if (params) {
      request.params = params
    }
    if (sessionId) {
      request.sessionId = sessionId
    }

    return new Promise((resolve, reject) => {
      this.pendingRequests.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      })
      this.ws?.send(JSON.stringify(request))
    })
  }

  async createPage(url: string, viewport: { width: number; height: number }): Promise<void> {
    // Create target
    const createResult = await this.send<{ targetId: string }>('Target.createTarget', {
      url: 'about:blank',
    })
    this.targetId = createResult.targetId

    // Attach to target
    const attachResult = await this.send<{ sessionId: string }>('Target.attachToTarget', {
      targetId: this.targetId,
      flatten: true,
    })
    this.sessionId = attachResult.sessionId

    // Enable required domains
    await Promise.all([
      this.send('Page.enable', {}, this.sessionId),
      this.send('DOM.enable', {}, this.sessionId),
    ])

    // Set viewport
    await this.send(
      'Emulation.setDeviceMetricsOverride',
      {
        width: viewport.width,
        height: viewport.height,
        deviceScaleFactor: 1,
        mobile: false,
      },
      this.sessionId,
    )

    // Navigate and wait for load
    await this.navigateAndWait(url)
  }

  private async navigateAndWait(url: string): Promise<void> {
    // Set up listener for domContentEventFired before navigating
    const loadPromise = new Promise<void>((resolve) => {
      // We'll just wait a bit since we don't have a full event system
      setTimeout(resolve, 1500)
    })

    await this.send('Page.navigate', { url }, this.sessionId)
    await loadPromise
  }

  async captureScreenshot(): Promise<{ base64: string; url: string }> {
    if (!this.sessionId) {
      throw new Error('No page created')
    }

    // Get current URL via Runtime.evaluate
    const urlResult = await this.send<{ result: { value: string } }>(
      'Runtime.evaluate',
      { expression: 'window.location.href' },
      this.sessionId,
    )

    // Capture screenshot
    const result = await this.send<{ data: string }>(
      'Page.captureScreenshot',
      { format: 'png' },
      this.sessionId,
    )

    return {
      base64: result.data,
      url: urlResult.result.value,
    }
  }

  async closePage(): Promise<void> {
    if (this.targetId) {
      try {
        await this.send('Target.closeTarget', { targetId: this.targetId })
      } catch {
        // Ignore close errors
      }
      this.targetId = undefined
      this.sessionId = undefined
    }
  }

  isConnected(): boolean {
    return this.connected && this.ws?.readyState === WebSocket.OPEN
  }

  hasPage(): boolean {
    return this.sessionId !== undefined
  }

  disconnect(): void {
    if (this.ws) {
      this.ws.close()
      this.ws = undefined
    }
    this.connected = false
    this.sessionId = undefined
    this.targetId = undefined
    this.pendingRequests.clear()
  }
}

// Singleton CDP client for screenshots
let screenshotClient: ScreenshotCDPClient | undefined

const PREVIEW_URL = 'http://localhost:3000'

/**
 * Ensure CDP client is connected
 */
async function ensureCDPConnected(cdpPort: number): Promise<ScreenshotCDPClient> {
  if (screenshotClient?.isConnected()) {
    return screenshotClient
  }

  logger.info({ cdpPort }, 'Connecting to browser via CDP for screenshot')
  screenshotClient = new ScreenshotCDPClient()
  await screenshotClient.connect(cdpPort)
  return screenshotClient
}

/**
 * Ensure page exists and is ready for screenshot
 */
async function ensurePageReady(client: ScreenshotCDPClient): Promise<void> {
  if (client.hasPage()) {
    return
  }

  logger.info({ url: PREVIEW_URL }, 'Creating page and navigating to preview server')
  await client.createPage(PREVIEW_URL, { width: 1024, height: 720 })
}

/**
 * Clean up screenshot client on error
 */
async function cleanupScreenshotClient(): Promise<void> {
  if (screenshotClient) {
    try {
      await screenshotClient.closePage()
    } catch {
      // Ignore close errors
    }
    screenshotClient.disconnect()
    screenshotClient = undefined
  }
}

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

      // Clean up screenshot CDP client if it exists
      await cleanupScreenshotClient()

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

  // POST /browser/screenshot - Capture screenshot of preview server and return base64
  app.post('/browser/screenshot', async (c) => {
    try {
      const server = getBrowserServer()

      // Auto-start browser if not running
      if (!server.isRunning()) {
        logger.info('Browser not running, starting for screenshot capture')
        await server.start()
      }

      // Get CDP port from server info
      const info = server.getInfo()
      const cdpPort = info.cdpPort
      if (!cdpPort) {
        throw new Error('Browser server has no CDP port configured')
      }

      // Ensure CDP client is connected
      const client = await ensureCDPConnected(cdpPort)

      // Ensure page is ready
      await ensurePageReady(client)

      // Capture screenshot
      logger.info('Capturing screenshot via CDP')
      const { base64: screenshotBase64, url: currentUrl } = await client.captureScreenshot()

      logger.info(
        { url: currentUrl, base64Length: screenshotBase64.length },
        'Screenshot captured successfully',
      )

      return c.json({
        success: true,
        screenshotBase64,
        url: currentUrl,
      })
    } catch (error) {
      logger.error({ err: error }, 'Failed to capture screenshot')
      await cleanupScreenshotClient()

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
