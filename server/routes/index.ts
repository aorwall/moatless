import type { FastifyInstance } from 'fastify'
import { registerEnvironmentRoutes } from './environments.js'
import { registerTaskRoutes } from './tasks.js'
import { registerWorkspaceRoutes } from './workspaces.js'

const API_PREFIX = '/api/v1'

export async function registerRoutes(app: FastifyInstance) {
  // Main API info endpoint
  app.get(`${API_PREFIX}`, async () => ({
    message: 'Moatless API',
    version: '1.0.0',
    endpoints: {
      health: '/health',
      tasks: `${API_PREFIX}/tasks`,
      environments: `${API_PREFIX}/environments`,
      workspaces: `${API_PREFIX}/workspaces`,
      events: `${API_PREFIX}/events`,
    },
  }))

  // Register API route groups with prefixes
  await app.register(registerTaskRoutes, { prefix: `${API_PREFIX}/tasks` })
  await app.register(registerEnvironmentRoutes, { prefix: `${API_PREFIX}/environments` })
  await app.register(registerWorkspaceRoutes, { prefix: `${API_PREFIX}/workspaces` })

  // Catch-all for client-side routing in production
  if (process.env.NODE_ENV === 'production') {
    app.get('/*', async (_request, reply) => {
      return reply.sendFile('index.html')
    })
  }
}
