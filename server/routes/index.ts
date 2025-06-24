import type { FastifyInstance } from 'fastify'
import { registerEnvironmentRoutes } from './environments.js'
import { registerTaskRoutes } from './tasks.js'
import { registerWorkspaceRoutes } from './workspaces.js'

export async function registerRoutes(app: FastifyInstance) {
  // Main API info endpoint
  app.get('/api', async () => ({
    message: 'Moatless API',
    version: '1.0.0',
    endpoints: {
      health: '/health',
      tasks: '/api/tasks',
      environments: '/api/environments',
      workspaces: '/api/workspaces',
      events: '/api/events',
    },
  }))

  // Register API route groups with prefixes
  await app.register(registerTaskRoutes, { prefix: '/api/tasks' })
  await app.register(registerEnvironmentRoutes, { prefix: '/api/environments' })
  await app.register(registerWorkspaceRoutes, { prefix: '/api/workspaces' })

  // Catch-all for client-side routing in production
  if (process.env.NODE_ENV === 'production') {
    app.get('/*', async (request, reply) => {
      return reply.sendFile('index.html')
    })
  }
}
