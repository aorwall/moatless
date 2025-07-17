import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import { registerCommandRoutes } from './commands.js'
import { registerEventRoutes } from './events.js'
import { registerPreviewEnvironmentRoutes } from './preview-environment.js'
import { registerRepositoryRoutes } from './repositories.js'
import { registerSettingsRoutes } from './settings.js'
import { registerTaskRoutes } from './tasks.js'
import { registerUploadRoutes } from './uploads.js'
import { registerWorkerRoutes } from './worker.js'
import { registerWorkspaceRoutes } from './workspace.js'

const API_PREFIX = '/api/v1'

const apiInfoResponseSchema = z.object({
  message: z.string(),
  version: z.string(),
  endpoints: z.object({
    health: z.string(),
    tasks: z.string(),
    repositories: z.string(),
    commands: z.string(),
    events: z.string(),
    settings: z.string(),
    worker: z.string(),
  }),
})

export async function registerRoutes(app: Hono<AppBindings>) {
  // Main API info endpoint
  app.get(
    `${API_PREFIX}`,
    describeRoute({
      description: 'Get API information and available endpoints',
      responses: {
        200: {
          description: 'API information and endpoint list',
          content: {
            'application/json': {
              schema: resolver(apiInfoResponseSchema),
            },
          },
        },
      },
    }),
    (c) => {
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
          worker: `${API_PREFIX}/worker`,
        },
      })
    },
  )

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
}
