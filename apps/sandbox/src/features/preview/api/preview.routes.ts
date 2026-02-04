import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { getRegistryOrUndefined } from '../../servers/services/server-registry'

/**
 * Register preview server routes
 *
 * The preview server process is managed externally by the shell script
 * and the Claude Code agent. These routes provide monitoring and proxying only.
 */
export function registerPreviewRoutes(app: Hono<AppBindings>) {
  // GET /server/status - Get preview server status
  app.get('/server/status', async (c) => {
    const monitor = getRegistryOrUndefined()?.get('preview')

    if (!monitor) {
      return c.json({
        status: 'stopped',
        running: false,
        configured: false,
      })
    }

    const status = await monitor.getStatus()
    return c.json({
      ...status,
      running: status.status === 'started',
      configured: true,
      port: await monitor.getConfiguredPort(),
    })
  })

  // GET /server/logs - Get preview server logs
  app.get('/server/logs', async (c) => {
    const monitor = getRegistryOrUndefined()?.get('preview')

    if (!monitor) {
      return c.json({ logs: '' })
    }

    const lines = c.req.query('lines')
    const logs = await monitor.getLogs(lines ? Number.parseInt(lines, 10) : undefined)
    return c.json({ logs })
  })

  // GET /server/logs/stream - Stream preview server logs via SSE
  app.get('/server/logs/stream', (c) => {
    const monitor = getRegistryOrUndefined()?.get('preview')

    if (!monitor) {
      return c.json({ error: 'Monitor not initialized' }, 404)
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
