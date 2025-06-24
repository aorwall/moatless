import path from 'node:path'
import { fileURLToPath } from 'node:url'
import cors from '@fastify/cors'
import helmet from '@fastify/helmet'
import staticPlugin from '@fastify/static'
import websocket from '@fastify/websocket'
import type { FastifyInstance } from 'fastify'
import { errorHandler } from './plugins/error-handler.js'
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
    origin: process.env.CORS_ORIGIN || true,
    credentials: true,
  })

  // WebSocket support
  await app.register(websocket)

  // Error handling
  app.setErrorHandler(errorHandler)

  // Serve client build in production
  if (process.env.NODE_ENV === 'production') {
    await app.register(staticPlugin, {
      root: path.join(__dirname, '../../client/dist'),
      prefix: '/',
    })
  }

  // Setup services
  const services = setupServices(app)
  app.decorate('services', services)

  // Register API routes
  await registerRoutes(app)

  // Health check
  app.get('/health', async () => ({
    status: 'ok',
    timestamp: new Date().toISOString(),
    version: process.env.npm_package_version || '1.0.0',
  }))
}
