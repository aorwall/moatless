import path from 'node:path'
import { fileURLToPath } from 'node:url'
import cors from '@fastify/cors'
import helmet from '@fastify/helmet'
import multipart from '@fastify/multipart'
import staticPlugin from '@fastify/static'
import type { FastifyInstance } from 'fastify'
import { errorHandler } from './plugins/error-handler.js'
import { setupValidation } from './plugins/validation.js'
import { registerRoutes } from './routes/index.js'
import { setupServices } from './services/index.js'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

export async function setupApp(app: FastifyInstance) {
  // Security
  await app.register(helmet, {
    contentSecurityPolicy: false, // Allow inline scripts for development
  })

  // CORS
  await app.register(cors, {
    origin: (origin, callback) => {
      // In development, allow all localhost origins
      if (process.env.NODE_ENV !== 'production' && origin) {
        const url = new URL(origin)
        if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
          callback(null, true)
          return
        }
      }

      // In production or for non-localhost origins, use configured origin
      const allowedOrigin = process.env.CORS_ORIGIN || true
      callback(null, allowedOrigin)
    },
    credentials: true,
  })

  // Validation
  await setupValidation(app)

  // Multipart/file uploads support
  await app.register(multipart, {
    limits: {
      fileSize: 10 * 1024 * 1024, // 10MB
      files: 10, // Maximum number of files
    },
  })

  // Error handling
  app.setErrorHandler(errorHandler)

  // Setup services
  const services = await setupServices(app)
  app.decorate('services', services)

  // Register API routes
  await registerRoutes(app)

  // Health check
  app.get('/health', async () => ({
    status: 'ok',
    timestamp: new Date().toISOString(),
    version: process.env.npm_package_version || '1.0.0',
  }))

  // Serve client build in production
  if (process.env.NODE_ENV === 'production') {
    // Register static files first
    await app.register(staticPlugin, {
      root: path.join(__dirname, '../../frontend/dist'),
      prefix: '/',
    })
    
    // Then register SPA fallback for non-API routes
    app.setNotFoundHandler(async (request, reply) => {
      // Skip API routes and actual files
      if (request.url.startsWith('/api') || request.url.startsWith('/health')) {
        reply.code(404).send({ error: 'Not Found' })
        return
      }

      return reply.sendFile('index.html')
    })
  }
}
