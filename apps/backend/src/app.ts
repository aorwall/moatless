import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { OpenAPIHono } from '@hono/zod-openapi'
import type { UserSession } from '@moatless/types'
import { apiReference } from '@scalar/hono-api-reference'
import { cors } from 'hono/cors'
import { secureHeaders } from 'hono/secure-headers'
import { serveStatic } from 'hono/serve-static'
import { errorHandler } from './middleware/error-handler.js'
import { githubAuth } from './middleware/github-auth.js'
import { logger as loggerMiddleware } from './middleware/logger.js'
import { requestId } from './middleware/request-id.js'
import { registerRoutes } from './routes/index.js'
import { createServiceContainer, type ServiceContainer } from './services/service-container.js'
import { logger } from './utils/logger.js'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

export type AppBindings = {
  Variables: {
    services: ServiceContainer
    requestId: string
    session?: UserSession
  }
}

export interface AppWithServices {
  app: OpenAPIHono<AppBindings>
  services: ServiceContainer
}

export async function createApp(): Promise<AppWithServices> {
  const app = new OpenAPIHono<AppBindings>()

  // Request ID middleware (must be first)
  app.use('*', requestId)

  // OpenTelemetry middleware (if enabled via APPLICATIONINSIGHTS_CONNECTION_STRING)
  if (process.env['APPLICATIONINSIGHTS_CONNECTION_STRING']) {
    try {
      const { otel } = await import('@hono/otel')
      app.use('*', otel())
      logger.info('OpenTelemetry middleware enabled - HTTP requests will be traced')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.warn(`Failed to load OpenTelemetry middleware: ${errorMessage}`)
    }
  }

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
        if (process.env['NODE_ENV'] !== 'production' && origin) {
          const url = new URL(origin)
          if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
            return origin
          }
        }

        // In production or for non-localhost origins, use configured origin
        return process.env['CORS_ORIGIN'] || origin || '*'
      },
      credentials: true,
    }),
  )

  // Setup services and inject into context
  const services = await createServiceContainer()
  app.use('*', async (c, next) => {
    c.set('services', services)
    await next()
  })

  // Authentication middleware
  // Use GitHub OAuth if enabled, otherwise no authentication
  if (process.env['GITHUB_OAUTH_ENABLED'] === 'true') {
    app.use('*', githubAuth)
  }

  // Error handling middleware
  app.onError(errorHandler)

  // Register API routes
  await registerRoutes(app)

  // Health check
  app.get('/health', (c) => {
    return c.json({
      status: 'ok',
      timestamp: new Date().toISOString(),
      version: process.env['npm_package_version'] || '1.0.0',
    })
  })

  // OpenAPI specification endpoint (automatically generated from routes)
  app.doc('/openapi.json', {
    openapi: '3.0.0',
    info: {
      title: 'Moatless API',
      version: '1.0.0',
      description:
        'API for the Moatless application - AI-powered task management and code generation',
    },
    servers: [
      {
        url: process.env['API_URL'] || 'http://localhost:8000',
        description:
          process.env['NODE_ENV'] === 'production' ? 'Production server' : 'Development server',
      },
    ],
  })

  // API documentation UI with Scalar
  app.get(
    '/docs',
    apiReference({
      theme: 'purple',
      url: '/openapi.json',
    }),
  )

  // Serve client build in production
  if (process.env['NODE_ENV'] === 'production') {
    // Serve static files
    const staticPath =
      process.env['STATIC_CLIENT_PATH'] || path.join(__dirname, '../../frontend/dist')
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
        onFound: (path: string, c) => {
          // Set proper MIME type for source maps
          if (path.endsWith('.js.map')) {
            c.header('Content-Type', 'application/json')
          } else if (path.endsWith('.css.map')) {
            c.header('Content-Type', 'application/json')
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
