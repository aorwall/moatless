import type { FastifyInstance } from 'fastify'
import { registerCommandRoutes } from './commands.js'
import { registerRepositoryRoutes } from './repositories.js'
import { registerEventRoutes } from './events.js'
import { registerTaskRoutes } from './tasks.js'
import { uploadRoutes } from './uploads.js'

const API_PREFIX = '/api/v1'

export async function registerRoutes(app: FastifyInstance) {
  // Main API info endpoint
  app.get(`${API_PREFIX}`, async () => ({
    message: 'Moatless API',
    version: '1.0.0',
    endpoints: {
      health: '/health',
      tasks: `${API_PREFIX}/tasks`,
      repositories: `${API_PREFIX}/repositories`,
      commands: `${API_PREFIX}/commands`,
      events: `${API_PREFIX}/events`,
    },
  }))

  // Register API route groups with prefixes
  await app.register(registerTaskRoutes, { prefix: `${API_PREFIX}/tasks` })
  await app.register(registerRepositoryRoutes, { prefix: `${API_PREFIX}/repositories` })
  await app.register(registerEventRoutes, { prefix: `${API_PREFIX}/events` })
  await app.register(uploadRoutes, { prefix: `${API_PREFIX}/uploads` })
  await app.register(registerCommandRoutes, { prefix: `${API_PREFIX}/commands` })

  // Catch-all for client-side routing in production
  if (process.env.NODE_ENV === 'production') {
    app.get('/*', async (_request, reply) => {
      return reply.sendFile('index.html')
    })
  }
}
