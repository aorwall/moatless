import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { PreviewServer } from '../runtime/preview-server'
import type { PreviewServerConfig } from '../types/config'

// Module-scoped preview server instance (one per sandbox)
let previewServer: PreviewServer | undefined

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

      // Stop existing server if running
      if (previewServer) {
        await previewServer.stop()
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

      previewServer = new PreviewServer(serverConfig)
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
    if (!previewServer) {
      return c.json({ logs: '' })
    }

    const lines = c.req.query('lines')
    const logs = previewServer.getLogs(lines ? Number.parseInt(lines, 10) : undefined)
    return c.json({ logs })
  })

  // GET /server/logs/stream - Stream preview server logs via SSE
  app.get('/server/logs/stream', (c) => {
    if (!previewServer) {
      return c.json({ error: 'No server running' }, 404)
    }

    return streamSSE(c, async (stream) => {
      for await (const logEvent of previewServer!.streamLogs()) {
        await stream.writeSSE({
          event: logEvent.type,
          data: JSON.stringify(logEvent.data),
        })
      }
    })
  })

  // GET /server/proxy/* - Proxy request through to running server
  app.all('/server/proxy/*', async (c) => {
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

      const result = await previewServer.proxyRequest({
        method: c.req.method,
        path: proxyPath,
        headers: c.req.raw.headers,
        body: c.req.method !== 'GET' && c.req.method !== 'HEAD' ? await c.req.text() : undefined,
      })

      // Convert Buffer to string if needed
      const body = Buffer.isBuffer(result.body) ? result.body.toString('utf-8') : result.body

      // Create response with proxied headers
      const responseHeaders = new Headers()
      result.headers.forEach((value, key) => {
        const lowerKey = key.toLowerCase()
        // Skip hop-by-hop headers and content-length (will be recalculated)
        if (
          !['transfer-encoding', 'connection', 'keep-alive', 'content-length'].includes(lowerKey)
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
}
