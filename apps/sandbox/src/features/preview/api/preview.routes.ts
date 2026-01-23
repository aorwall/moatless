import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { PreviewServer } from '../runtime/preview-server'
import { getRegistryOrUndefined } from '../services/preview-server-registry'
import type { PreviewServerConfig } from '../types/config'

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

  // /preview/* - Delegate to PreviewServer.handleProxyRequest
  app.all('/preview/*', (c) => {
    const registry = getRegistryOrUndefined()
    const previewServer = registry?.getServer()

    if (!previewServer) {
      return c.json({ error: 'No preview server running' }, 503)
    }

    const status = previewServer.getStatus()
    if (status.status !== 'started') {
      return c.json({ error: `Preview server not ready: ${status.status}` }, 503)
    }

    const proxyPath = c.req.path.replace('/preview', '') || '/'
    return previewServer.handleProxyRequest(c.req.raw, proxyPath)
  })
}
