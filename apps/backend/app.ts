import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { Hono } from 'hono'
import { cors } from 'hono/cors'
import { secureHeaders } from 'hono/secure-headers'
import { serveStatic } from 'hono/serve-static'
import { errorHandler } from './middleware/error-handler.js'
import { logger as loggerMiddleware } from './middleware/logger.js'
import { requestId } from './middleware/request-id.js'
import { registerRoutes } from './routes/index.js'
import { type ServiceContainer, createServiceContainer } from './services/service-container.js'
import { logger } from './utils/logger.js'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

export type AppBindings = {
  Variables: {
    services: ServiceContainer
    requestId: string
  }
}

export interface AppWithServices {
  app: Hono<AppBindings>
  services: ServiceContainer
}

export async function createApp(): Promise<AppWithServices> {
  const app = new Hono<AppBindings>()

  // Request ID middleware (must be first)
  app.use('*', requestId)

  // Logger middleware
  app.use('*', loggerMiddleware)

  // Security headers
  app.use('*', secureHeaders())

  // CORS
  app.use(
    '*',
    cors({
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
    }),
  )

  // Error handling middleware
  app.onError(errorHandler)

  // Setup services and inject into context
  const dataDir = process.env.DATA_DIR || path.join(__dirname, '../../../data')
  const services = await createServiceContainer({ dataDir })
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
    const staticPath = process.env.STATIC_CLIENT_PATH || path.join(__dirname, '../../frontend/dist')
    app.use(
      '/*',
      serveStatic({
        root: staticPath,
        getContent: async (path: string) => {
          try {
            const fs = await import('node:fs/promises')
            return await fs.readFile(path)
          } catch {
            return null
          }
        },
      }),
    )

    // SPA fallback for non-API routes
    app.get('*', async (c) => {
      // Skip API routes
      if (c.req.path.startsWith('/api') || c.req.path.startsWith('/health')) {
        return c.notFound()
      }

      // Serve the actual built index.html
      const fs = await import('node:fs/promises')
      const indexPath = path.join(staticPath, 'index.html')
      try {
        const indexHtml = await fs.readFile(indexPath, 'utf-8')
        return c.html(indexHtml)
      } catch (error) {
        logger.error('Failed to read index.html', error)
        return c.notFound()
      }
    })
  } else {
    // In development mode, return 404 for unknown routes
    app.get('*', (c) => {
      return c.notFound()
    })
  }

  return { app, services }
}
