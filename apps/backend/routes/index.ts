import type { Hono } from 'hono'
import type { AppBindings } from '../app.js'
import { registerCommandRoutes } from './commands.js'
import { registerEventRoutes } from './events.js'
import { registerRepositoryRoutes } from './repositories.js'
import { registerTaskRoutes } from './tasks.js'
import { registerUploadRoutes } from './uploads.js'

const API_PREFIX = '/api/v1'

export async function registerRoutes(app: Hono<AppBindings>) {
  // Main API info endpoint
  app.get(`${API_PREFIX}`, (c) => {
    return c.json({
      message: 'Moatless API',
      version: '1.0.0',
      endpoints: {
        health: '/health',
        tasks: `${API_PREFIX}/tasks`,
        repositories: `${API_PREFIX}/repositories`,
        commands: `${API_PREFIX}/commands`,
        events: `${API_PREFIX}/events`,
      },
    })
  })

  // Register API route groups with prefixes
  await registerTaskRoutes(app, `${API_PREFIX}/tasks`)
  await registerRepositoryRoutes(app, `${API_PREFIX}/repositories`)
  await registerEventRoutes(app, `${API_PREFIX}/events`)
  await registerUploadRoutes(app, `${API_PREFIX}/uploads`)
  await registerCommandRoutes(app, `${API_PREFIX}/commands`)
}
