import type { OpenAPIHono } from '@hono/zod-openapi'
import type { AppBindings } from '../app'

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

  // Dynamic imports to avoid circular dependencies during module loading
  const { registerAuthRoutes } = await import('../features/auth/api/auth.routes.js')
  const { registerApiKeyRoutes } = await import('../features/auth/api/api-key.routes.js')
  const { registerEventRoutes } = await import('../features/events/index.js')
  const { registerFigmaRoutes } = await import('../features/figma/index.js')
  const { registerRepositoryRoutes } = await import('../features/repositories/index.js')
  const { registerTaskRoutes, registerWorkerRoutes } = await import('../features/tasks/index.js')
  const { registerFileRoutes, registerWorkspaceRoutes: registerWorkspaceManagementRoutes } =
    await import('../features/workspace/index.js')
  const { registerPreviewRoutes } = await import('../features/preview/index.js')
  const { registerAgentRoutes, registerMessagesRoutes } = await import(
    '../features/agents/index.js'
  )
  const { registerCommandRoutes } = await import('../features/agents/api/commands.routes.js')
  const { registerFeatureFlagsRoutes } = await import('./feature-flags.js')
  const { registerSettingsRoutes } = await import('./settings.js')
  const { registerFileStorageRoutes } = await import('../features/file-storage/index.js')
  // TODO: usage.ts file is missing - need to restore or remove
  // const { registerUsageRoutes } = await import('./usage.js')

  // Register authentication routes
  // Routes handle dev vs prod mode internally
  await registerAuthRoutes(app)
  await registerApiKeyRoutes(app)

  // Register API route groups with prefixes
  await registerTaskRoutes(app)

  // NEW: Use new workspace feature routes for all workspace operations
  registerFileRoutes(app) // File operations (list, read, write, search, git-diff, file-tree, execute)
  registerWorkspaceManagementRoutes(app) // Workspace management (setup, status)
  registerPreviewRoutes(app) // Preview server operations (start, stop, restart, status, logs)
  registerAgentRoutes(app) // Agent operations (start, stop, stream)
  registerMessagesRoutes(app) // Messages routes from agents feature

  await registerRepositoryRoutes(app)
  await registerEventRoutes(app, `${API_PREFIX}/events`)
  await registerFileStorageRoutes(app)
  await registerCommandRoutes(app)
  await registerWorkerRoutes(app)
  await registerFigmaRoutes(app)

  // Register settings routes
  await registerSettingsRoutes(app)

  // Register usage routes
  // TODO: usage.ts file is missing - need to restore or remove
  // await registerUsageRoutes(app)

  // Register feature flags routes
  registerFeatureFlagsRoutes(app)
}
