import type { OpenAPIHono } from '@hono/zod-openapi'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import { registerAuthRoutes } from './auth.js'
import { registerCommandRoutes } from './commands.js'
import { registerEventRoutes } from './events.js'
import { registerPreviewEnvironmentRoutes } from './preview-environment.js'
import { registerRepositoryRoutes } from './repositories.js'
import { registerSettingsRoutes } from './settings.js'
import { registerTaskRoutes } from './tasks.js'
import { registerUploadRoutes } from './uploads.js'
import { registerUsageRoutes } from './usage.js'
import { registerWorkerRoutes } from './worker.js'
import { registerWorkspaceRoutes } from './workspace.js'

const API_PREFIX = '/api/v1'

const _apiInfoResponseSchema = z.object({
  message: z.string(),
  version: z.string(),
  endpoints: z.object({
    health: z.string(),
    tasks: z.string(),
    repositories: z.string(),
    commands: z.string(),
    events: z.string(),
    settings: z.string(),
    usage: z.string(),
    worker: z.string(),
  }),
})

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
      },
    })
  })

  // Register authentication routes (if GitHub OAuth is enabled)
  if (process.env.GITHUB_OAUTH_ENABLED === 'true') {
    await registerAuthRoutes(app, '/auth')
  }

  // Register API route groups with prefixes
  await registerTaskRoutes(app, `${API_PREFIX}/tasks`)
  await registerWorkspaceRoutes(app, `${API_PREFIX}/tasks`)
  await registerPreviewEnvironmentRoutes(app, `${API_PREFIX}/tasks`)
  await registerRepositoryRoutes(app, `${API_PREFIX}/repositories`)
  await registerEventRoutes(app, `${API_PREFIX}/events`)
  await registerUploadRoutes(app, `${API_PREFIX}/uploads`)
  await registerCommandRoutes(app, `${API_PREFIX}/commands`)
  await registerWorkerRoutes(app, `${API_PREFIX}/worker`)

  // Register settings routes
  await registerSettingsRoutes(app, `${API_PREFIX}/settings`)

  // Register usage routes
  await registerUsageRoutes(app, `${API_PREFIX}/usage`)
}
