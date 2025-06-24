import cors from '@fastify/cors'
import type { FastifyInstance } from 'fastify'

export async function setupApp(app: FastifyInstance) {
  // Register plugins
  await app.register(cors, {
    origin: process.env.CORS_ORIGIN || true,
  })

  // Health check
  app.get('/health', async () => ({ status: 'ok' }))

  // Basic API route
  app.get('/api', async () => ({
    message: 'Moatless API',
    version: '1.0.0',
  }))
}
