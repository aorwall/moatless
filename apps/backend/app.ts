import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { Hono } from 'hono'
import { cors } from 'hono/cors'
import { secureHeaders } from 'hono/secure-headers'
import { serveStatic } from 'hono/serve-static'
import { errorHandler } from './middleware/error-handler.js'
import { requestId } from './middleware/request-id.js'
import { logger } from './middleware/logger.js'
import { registerRoutes } from './routes/index.js'
import { setupServices, type Services } from './services/index.js'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

export type AppBindings = {
  Variables: {
    services: Services
    requestId: string
  }
}

export async function createApp(): Promise<Hono<AppBindings>> {
  const app = new Hono<AppBindings>()

  // Request ID middleware (must be first)
  app.use('*', requestId)

  // Logger middleware
  app.use('*', logger)

  // Security headers
  app.use('*', secureHeaders())

  // CORS
  app.use('*', cors({
    origin: (origin) => {
      // In development, allow all localhost origins
      if (process.env.NODE_ENV !== 'production' && origin) {
        const url = new URL(origin)
        if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
          return origin
        }
      }

      // In production or for non-localhost origins, use configured origin
      return process.env.CORS_ORIGIN || origin || '*'
    },
    credentials: true,
  }))


  // Error handling middleware
  app.onError(errorHandler)

  // Setup services and inject into context
  const services = await setupServices()
  app.use('*', async (c, next) => {
    c.set('services', services)
    await next()
  })

  // Register API routes
  await registerRoutes(app)

  // Health check
  app.get('/health', (c) => {
    return c.json({
      status: 'ok',
      timestamp: new Date().toISOString(),
      version: process.env.npm_package_version || '1.0.0',
    })
  })

  // Serve client build in production
  if (process.env.NODE_ENV === 'production') {
    // Serve static files
    const staticPath = path.join(__dirname, '../../frontend/dist')
    app.use('/*', serveStatic({
      root: staticPath,
      getContent: async (path: string) => {
        try {
          const fs = await import('node:fs/promises')
          const content = await fs.readFile(path)
          return content
        } catch {
          return null
        }
      }
    }))
    
    // SPA fallback for non-API routes
    app.get('*', (c) => {
      // Skip API routes
      if (c.req.path.startsWith('/api') || c.req.path.startsWith('/health')) {
        return c.notFound()
      }
      
      return c.html(`<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Moatless Swarm UI</title>
    <script type="module" crossorigin src="/assets/index.js"></script>
    <link rel="stylesheet" crossorigin href="/assets/index.css">
</head>
<body>
    <div id="root"></div>
</body>
</html>`)
    })
  } else {
    // In development mode, return 404 for unknown routes
    app.get('*', (c) => {
      return c.notFound()
    })
  }

  return app
}
