import type { OpenAPIHono } from '@hono/zod-openapi'
import type { AppBindings } from '../app.js'
import { registerAuthRoutes } from './auth.js'
import { registerCommandRoutes } from './commands.js'
import { registerEventRoutes } from './events.js'
import { registerFeatureFlagsRoutes } from './feature-flags.js'
import { registerFigmaRoutes } from './figma.js'
import { registerPreviewEnvironmentRoutes } from './preview-environment.js'
import { registerRepositoryRoutes } from './repositories.js'
import { registerSettingsRoutes } from './settings.js'
import { registerTaskRoutes } from './tasks.js'
import { registerUploadRoutes } from './uploads.js'
import { registerUsageRoutes } from './usage.js'
import { registerWorkerRoutes } from './worker.js'
import { registerWorkspaceRoutes } from './workspace.js'

const API_PREFIX = '/api/v1'

export async function registerRoutes(app: OpenAPIHono<AppBindings>) {
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
        settings: `${API_PREFIX}/settings`,
        usage: `${API_PREFIX}/usage`,
        worker: `${API_PREFIX}/worker`,
        figma: `${API_PREFIX}/figma`,
        featureFlags: `${API_PREFIX}/feature-flags`,
      },
    })
  })

  // Register authentication routes
  // Routes handle dev vs prod mode internally
  await registerAuthRoutes(app)

  // Register API route groups with prefixes
  await registerTaskRoutes(app, `${API_PREFIX}/tasks`)
  await registerWorkspaceRoutes(app)
  await registerPreviewEnvironmentRoutes(app)
  await registerRepositoryRoutes(app)
  await registerEventRoutes(app, `${API_PREFIX}/events`)
  await registerUploadRoutes(app)
  await registerCommandRoutes(app)
  await registerWorkerRoutes(app)
  await registerFigmaRoutes(app)

  // Register settings routes
  await registerSettingsRoutes(app)

  // Register usage routes
  await registerUsageRoutes(app)

  // Register feature flags routes
  registerFeatureFlagsRoutes(app)
}
