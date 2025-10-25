import type { OpenAPIHono } from '@hono/zod-openapi'
import type { AppBindings } from '../app'
import { registerAuthRoutes } from '../features/auth/api/auth.routes'
import { registerEventRoutes } from '../features/events/index'
import { registerRepositoryRoutes } from '../features/repositories/index'
import { registerCommandRoutes } from './commands'
import { registerFeatureFlagsRoutes } from './feature-flags'
import { registerFigmaRoutes } from './figma'
import { registerPreviewEnvironmentRoutes } from './preview-environment'
import { registerSettingsRoutes } from './settings'
import { registerTaskRoutes } from './tasks'
import { registerTemplateRoutes } from './templates'
import { registerUploadRoutes } from './uploads'
import { registerUsageRoutes } from './usage'
import { registerWorkerRoutes } from './worker'
import { registerWorkspaceRoutes } from './workspace'

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
        templates: `${API_PREFIX}/templates`,
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
  await registerTemplateRoutes(app)

  // Register settings routes
  await registerSettingsRoutes(app)

  // Register usage routes
  await registerUsageRoutes(app)

  // Register feature flags routes
  registerFeatureFlagsRoutes(app)
}
