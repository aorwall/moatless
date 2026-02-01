import { Hono } from 'hono'
import type { SandboxConfig } from './config/environment'
// Feature route registrations
import { registerAgentRoutes } from './features/agents'
import type { MessagePollingConsumer } from './features/agents/services/message-polling-consumer'
import type { SandboxEventService } from './features/events/services/sandbox-event.service'
import { registerFileRoutes } from './features/files'
import { registerGitRoutes } from './features/git'
import { registerPreviewRoutes } from './features/preview'
import { registerServerRoutes } from './features/servers'
import { registerWorkspaceRoutes } from './features/workspace'
import { loggerMiddleware } from './middleware/logger'
import { requestIdMiddleware } from './middleware/request-id'

/**
 * App bindings for Hono context variables
 */
export type AppBindings = {
  Variables: {
    requestId: string
    config: SandboxConfig
    workspaceDir: string
    eventService?: SandboxEventService
    messageConsumer?: MessagePollingConsumer
  }
}

/**
 * Options for creating the app
 */
interface CreateAppOptions {
  eventService?: SandboxEventService
  messageConsumer?: MessagePollingConsumer
}

/**
 * Create the Hono application with all routes and middleware
 * Uses feature-based route composition for maintainability
 */
export function createApp(config: SandboxConfig, options?: CreateAppOptions) {
  const app = new Hono<AppBindings>()
  const eventService = options?.eventService
  const messageConsumer = options?.messageConsumer

  // Middleware
  app.use('*', requestIdMiddleware)
  app.use('*', loggerMiddleware)
  app.use('*', async (c, next) => {
    c.set('config', config)
    c.set('workspaceDir', config.workspaceDir)
    if (eventService) {
      c.set('eventService', eventService)
    }
    if (messageConsumer) {
      c.set('messageConsumer', messageConsumer)
    }
    await next()
  })

  // Health check endpoint (root level)
  app.get('/health', (c) =>
    c.json({
      status: 'ok' as const,
      mode: 'sandbox',
      workspaceDir: config.workspaceDir,
      taskId: config.taskId,
    }),
  )

  // Register feature routes
  registerWorkspaceRoutes(app)
  registerFileRoutes(app)
  registerGitRoutes(app)
  registerAgentRoutes(app)
  registerPreviewRoutes(app)
  registerServerRoutes(app)

  return app
}
