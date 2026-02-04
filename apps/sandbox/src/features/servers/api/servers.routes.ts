import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { getRegistryOrUndefined } from '../services/server-registry'

/**
 * Register server management routes
 *
 * Provides a multi-server API alongside the existing /server/* routes
 * for backward compatibility.
 */
export function registerServerRoutes(app: Hono<AppBindings>) {
  // GET /servers - List all servers with status
  app.get('/servers', async (c) => {
    const registry = getRegistryOrUndefined()

    if (!registry) {
      return c.json({ servers: [] })
    }

    return c.json({ servers: await registry.getAllStatuses() })
  })

  // GET /servers/:name/status - Get specific server status
  app.get('/servers/:name/status', async (c) => {
    const registry = getRegistryOrUndefined()
    const name = c.req.param('name')

    if (!registry) {
      return c.json({ error: 'Server registry not initialized' }, 503)
    }

    const monitor = registry.get(name)
    if (!monitor) {
      return c.json({ error: `Server '${name}' not found` }, 404)
    }

    const status = await monitor.getStatus()
    return c.json({
      ...status,
      running: status.status === 'started',
      configured: true,
      port: await monitor.getConfiguredPort(),
    })
  })

  // GET /servers/:name/logs - Get server logs
  app.get('/servers/:name/logs', async (c) => {
    const registry = getRegistryOrUndefined()
    const name = c.req.param('name')

    if (!registry) {
      return c.json({ error: 'Server registry not initialized' }, 503)
    }

    const monitor = registry.get(name)
    if (!monitor) {
      return c.json({ error: `Server '${name}' not found` }, 404)
    }

    const lines = c.req.query('lines')
    const logs = await monitor.getLogs(lines ? Number.parseInt(lines, 10) : undefined)
    return c.json({ logs })
  })

  // GET /servers/:name/logs/stream - Stream server logs via SSE
  app.get('/servers/:name/logs/stream', (c) => {
    const registry = getRegistryOrUndefined()
    const name = c.req.param('name')

    if (!registry) {
      return c.json({ error: 'Server registry not initialized' }, 503)
    }

    const monitor = registry.get(name)
    if (!monitor) {
      return c.json({ error: `Server '${name}' not found` }, 404)
    }

    return streamSSE(c, async (stream) => {
      for await (const logEvent of monitor.streamLogs()) {
        await stream.writeSSE({
          event: logEvent.type,
          data: JSON.stringify(logEvent.data),
        })
      }
    })
  })
}
