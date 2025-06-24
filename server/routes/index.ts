import type { FastifyInstance } from 'fastify'

export async function registerRoutes(app: FastifyInstance) {
  // API routes will be registered here
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

  // TODO: Register task routes
  // TODO: Register environment routes
  // TODO: Register workspace routes
  // TODO: Register WebSocket routes
}
