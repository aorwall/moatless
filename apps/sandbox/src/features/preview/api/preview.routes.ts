import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { injectScriptIntoHtml } from '../inspector/html-injector'
import { getPreviewInspectorScript } from '../inspector/inspector-script'
import { PreviewServer } from '../runtime/preview-server'
import { getRegistryOrUndefined } from '../services/preview-server-registry'
import type { PreviewServerConfig } from '../types/config'
import { isHtmlResponse } from '../utils/proxy-core'

/**
 * Register preview server routes
 */
export function registerPreviewRoutes(app: Hono<AppBindings>) {
  // POST /server/start - Start preview server
  app.post('/server/start', async (c) => {
    try {
      const body = await c.req.json<{
        serverCommands: string[]
        setupCommands?: string[]
        envVars?: Record<string, string>
        startupTimeoutSeconds?: number
        enableInspector?: boolean
      }>()

      if (!body.serverCommands || body.serverCommands.length === 0) {
        return c.json({ success: false, error: 'serverCommands is required' }, 400)
      }

      const workspaceDir = c.get('workspaceDir')
      const config = c.get('config')
      const registry = getRegistryOrUndefined()

      // Stop existing server if running (via registry or direct reference)
      const existingServer = registry?.getServer()
      if (existingServer) {
        await existingServer.stop()
      }

      const serverConfig: PreviewServerConfig = {
        serverId: config.taskId,
        workspacePath: workspaceDir,
        serverCommands: body.serverCommands,
        setupCommands: body.setupCommands,
        envVars: body.envVars,
        startupTimeoutSeconds: body.startupTimeoutSeconds,
        enableInspector: body.enableInspector,
      }

      const previewServer = new PreviewServer(serverConfig)

      // Register with registry if available
      if (registry) {
        registry.setServer(previewServer)
      }

      await previewServer.start()

      const status = previewServer.getStatus()
      return c.json({
        success: status.status === 'started',
        status: status.status,
        internalUrl: status.internalUrl,
        externalUrl: status.externalUrl,
        port: previewServer.getPort(),
      })
    } catch (error) {
      logger.error({ err: error }, 'Failed to start preview server')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // POST /server/stop - Stop preview server
  app.post('/server/stop', async (c) => {
    try {
      const registry = getRegistryOrUndefined()
      const previewServer = registry?.getServer()

      if (!previewServer) {
        return c.json({ success: true, message: 'No server running' })
      }

      const stopped = await previewServer.stop()
      return c.json({ success: true, stopped })
    } catch (error) {
      logger.error({ err: error }, 'Failed to stop preview server')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // GET /server/status - Get preview server status
  app.get('/server/status', (c) => {
    const registry = getRegistryOrUndefined()
    const previewServer = registry?.getServer()

    if (!previewServer) {
      return c.json({
        status: 'stopped',
        running: false,
        configured: false,
      })
    }

    const status = previewServer.getStatus()
    return c.json({
      ...status,
      running: status.status === 'started',
      configured: true,
      port: previewServer.getPort(),
    })
  })

  // GET /server/logs - Get preview server logs
  app.get('/server/logs', (c) => {
    const registry = getRegistryOrUndefined()
    const previewServer = registry?.getServer()

    if (!previewServer) {
      return c.json({ logs: '' })
    }

    const lines = c.req.query('lines')
    const logs = previewServer.getLogs(lines ? Number.parseInt(lines, 10) : undefined)
    return c.json({ logs })
  })

  // GET /server/logs/stream - Stream preview server logs via SSE
  app.get('/server/logs/stream', (c) => {
    const registry = getRegistryOrUndefined()
    const previewServer = registry?.getServer()

    if (!previewServer) {
      return c.json({ error: 'No server running' }, 404)
    }

    // Capture reference for use in async callback (avoids non-null assertion)
    const server = previewServer
    return streamSSE(c, async (stream) => {
      for await (const logEvent of server.streamLogs()) {
        await stream.writeSSE({
          event: logEvent.type,
          data: JSON.stringify(logEvent.data),
        })
      }
    })
  })

  // GET /server/proxy/* - Proxy request through to running server (legacy route)
  app.all('/server/proxy/*', async (c) => {
    const registry = getRegistryOrUndefined()
    const previewServer = registry?.getServer()

    if (!previewServer) {
      return c.json({ error: 'No server running' }, 503)
    }

    const status = previewServer.getStatus()
    if (status.status !== 'started') {
      return c.json({ error: `Server not ready: ${status.status}` }, 503)
    }

    try {
      // Extract path after /server/proxy
      const proxyPath = c.req.path.replace('/server/proxy', '') || '/'

      // Strip Accept-Encoding to prevent compressed responses
      // (Bun's fetch auto-decompresses, but headers would still indicate compression)
      const requestHeaders = new Headers(c.req.raw.headers)
      requestHeaders.delete('accept-encoding')

      const result = await previewServer.proxyRequest({
        method: c.req.method,
        path: proxyPath,
        headers: requestHeaders,
        body: c.req.method !== 'GET' && c.req.method !== 'HEAD' ? await c.req.text() : undefined,
      })

      // Convert Buffer to string if needed
      const body = Buffer.isBuffer(result.body) ? result.body.toString('utf-8') : result.body

      // Create response with proxied headers
      const responseHeaders = new Headers()
      result.headers.forEach((value, key) => {
        const lowerKey = key.toLowerCase()
        // Skip hop-by-hop headers, content-length (will be recalculated),
        // and content-encoding (Bun auto-decompresses, body is already decoded)
        if (
          ![
            'transfer-encoding',
            'connection',
            'keep-alive',
            'content-length',
            'content-encoding',
          ].includes(lowerKey)
        ) {
          responseHeaders.set(key, value)
        }
      })

      return new Response(body, {
        status: result.status,
        headers: responseHeaders,
      })
    } catch (error) {
      logger.error({ err: error }, 'Proxy request failed')
      return c.json(
        {
          error: error instanceof Error ? error.message : String(error),
        },
        502,
      )
    }
  })

  /**
   * /preview/* - Main preview proxy endpoint (forwarded from backend)
   *
   * This endpoint receives requests forwarded from the backend's HostnameProxyService.
   * It handles:
   * 1. Proxying requests to the internal preview server (localhost:3000)
   * 2. Injecting inspector script into HTML responses (only HTML is buffered)
   * 3. Streaming non-HTML responses directly for minimal overhead
   *
   * Security: Sandbox pods are only reachable from backend (internal K8s network).
   * No session validation needed here - network isolation IS the security.
   *
   * Performance: Uses streaming with conditional buffering.
   * - Index HTML only: buffered for script injection (/, /index.html)
   * - All other responses: streamed directly without buffering
   */
  app.all('/preview/*', async (c) => {
    const registry = getRegistryOrUndefined()
    const previewServer = registry?.getServer()
    const config = c.get('config')

    if (!previewServer) {
      return c.json({ error: 'No preview server running' }, 503)
    }

    const status = previewServer.getStatus()
    if (status.status !== 'started') {
      return c.json({ error: `Preview server not ready: ${status.status}` }, 503)
    }

    try {
      // Extract path after /preview
      const proxyPath = c.req.path.replace('/preview', '') || '/'
      const port = previewServer.getPort()
      const url = `http://localhost:${port}${proxyPath}`

      // Prepare request headers - strip Accept-Encoding to get uncompressed responses
      const requestHeaders = new Headers(c.req.raw.headers)
      requestHeaders.delete('accept-encoding')
      // Remove host header to avoid issues with virtual hosts
      requestHeaders.delete('host')

      // Forward request to internal preview server
      const response = await fetch(url, {
        method: c.req.method,
        headers: requestHeaders,
        body: c.req.method !== 'GET' && c.req.method !== 'HEAD' ? c.req.raw.body : undefined,
        // @ts-expect-error - Bun supports duplex for request body streaming
        duplex: 'half',
      })

      // Check Content-Type header to determine if we need to buffer for injection
      // For SPAs, only inject on index/document requests (/, /index.html, or routes without extensions)
      const contentType = response.headers.get('content-type')
      const isIndexRequest =
        proxyPath === '/' || proxyPath === '/index.html' || proxyPath.endsWith('/index.html')
      const shouldInjectScript =
        c.req.method === 'GET' && isHtmlResponse(contentType) && isIndexRequest

      if (shouldInjectScript) {
        // HTML: Buffer body, inject script, return modified response
        const body = await response.text()

        // Get inspector script with task ID
        const scriptContent = getPreviewInspectorScript(config.taskId)

        // Inject script into HTML
        const injectedBody = injectScriptIntoHtml(body, scriptContent)

        // Create response headers for modified HTML
        const responseHeaders = new Headers()
        response.headers.forEach((value, key) => {
          const lowerKey = key.toLowerCase()
          // Skip hop-by-hop headers and content-length (will be recalculated)
          if (
            !['transfer-encoding', 'connection', 'keep-alive', 'content-length'].includes(lowerKey)
          ) {
            responseHeaders.set(key, value)
          }
        })

        // Update headers for modified HTML content
        responseHeaders.delete('etag')
        responseHeaders.delete('last-modified')
        responseHeaders.set('cache-control', 'no-cache, no-store, must-revalidate')
        responseHeaders.set('pragma', 'no-cache')
        responseHeaders.set('expires', '0')

        // Remove headers that would prevent iframe embedding
        responseHeaders.delete('x-frame-options')
        responseHeaders.delete('content-security-policy')

        logger.debug(
          { path: proxyPath, taskId: config.taskId },
          'Injected inspector script into HTML response',
        )

        return new Response(injectedBody, {
          status: response.status,
          headers: responseHeaders,
        })
      }

      // Non-HTML: Stream response body directly without buffering
      // This is the fast path for assets (JS, CSS, images, etc.)
      return new Response(response.body, {
        status: response.status,
        headers: response.headers,
      })
    } catch (error) {
      logger.error({ err: error, path: c.req.path }, 'Preview proxy request failed')
      return c.json(
        {
          error: error instanceof Error ? error.message : String(error),
        },
        502,
      )
    }
  })
}
