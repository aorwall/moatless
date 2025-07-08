import type { Hono } from 'hono'
import type { AppBindings } from '../app.js'

export function registerWorkerRoutes(app: Hono<AppBindings>) {
  // Middleware to authenticate worker requests
  const authenticateWorker = async (c: any, next: any) => {
    const authHeader = c.req.header('Authorization')
    if (!authHeader?.startsWith('Bearer ')) {
      return c.json({ error: 'Unauthorized' }, 401)
    }

    // const apiKey = authHeader.slice(7)
    // TODO: Validate API key against registered workers

    await next()
  }

  // Event forwarding endpoint
  app.post('/api/v1/worker/events', authenticateWorker, async (c) => {
    const event = await c.req.json()

    // Forward to event service
    await c.get('services').event.emitEvent(event)

    return c.json({ success: true })
  })

  // Worker registration endpoint
  app.post('/api/v1/worker/register', async (c) => {
    await c.req.json()

    // TODO: Register worker in worker service

    return c.json({
      success: true,
      apiKey: 'generated-api-key',
    })
  })
}
