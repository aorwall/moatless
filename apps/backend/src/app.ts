import 'reflect-metadata'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { OpenAPIHono } from '@hono/zod-openapi'
import type { UserSession } from '@moatless/types'
import { apiReference } from '@scalar/hono-api-reference'
import { except } from 'hono/combine'
import { cors } from 'hono/cors'
import { secureHeaders } from 'hono/secure-headers'
import { serveStatic } from 'hono/serve-static'
import { container } from 'tsyringe'
import { errorHandler } from './middleware/error-handler.js'
import { logger as loggerMiddleware } from './middleware/logger.js'
import { requestId } from './middleware/request-id.js'
import { registerRoutes } from './routes/index.js'
import { auth } from './shared/http/middleware/auth.middleware.js'
import { sessionContext } from './shared/http/middleware/session-context.middleware.js'
import type { StorageBackend } from './storage/index.js'
import { logger } from './utils/logger.js'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

/**
 * Service container interface containing all application services.
 * Services are resolved via TSyringe dependency injection.
 */
export interface ServiceContainer {
  agent: import('./services/agent.service.js').AgentService
  auth: import('./features/auth/services/auth.service.js').AuthService
  claude: import('./agents/claude-code/claude.service.js').ClaudeService
  command: import('./services/command.service.js').CommandService
  docker: import('./services/docker.service.js').DockerService
  event: import('./services/event.service.js').EventService
  figma: import('./services/figma.service.js').FigmaService
  github: import('./services/github.service.js').GitHubService
  mcp: import('./services/mcp.service.js').McpService
  message: import('./services/message.service.js').MessageService
  netlify: import('./services/netlify.service.js').NetlifyService
  previewRegistry: import('@moatless/preview-server').PreviewServerRegistry
  repository: import('./services/repository.service.js').RepositoryService
  screenshot: import('./services/screenshot.service.js').ScreenshotService
  settings: import('./services/settings.service.js').SettingsService
  storage: StorageBackend
  task: import('./services/task.service.js').TaskService
  tool: import('./services/tool.service.js').ToolService
  upload: import('./services/upload.service.js').UploadService
  workspace: import('./services/workspace.service.js').WorkspaceService
}

export type AppBindings = {
  Variables: {
    services: ServiceContainer
    requestId: string
    session?: UserSession
  }
}

export interface AppWithServices {
  app: OpenAPIHono<AppBindings>
  services: ServiceContainer
}

/**
 * Register storage backend in TSyringe container based on STORAGE_TYPE env var
 */
async function registerStorageBackend(): Promise<void> {
  const storageType = process.env['STORAGE_TYPE']
  const authProvider = process.env['AUTH_PROVIDER']

  // Import storage classes and utilities
  const { getDataDir } = await import('./utils/data-dir.js')
  const { FileStorage, AuthorizedFileStorage, AuthorizedGroupFileStorage } = await import(
    './storage/index.js'
  )
  const { promises: fs } = await import('node:fs')

  const dataDir = getDataDir()
  await fs.mkdir(dataDir, { recursive: true })

  // Register storage with conditional logic based on STORAGE_TYPE
  if (storageType === 'user' && authProvider) {
    container.register('Storage', {
      useValue: new AuthorizedFileStorage(dataDir),
    })
    logger.info('TSyringe: Using user-scoped storage (AuthorizedFileStorage)')
  } else if (storageType === 'group' && authProvider) {
    container.register('Storage', {
      useValue: new AuthorizedGroupFileStorage(dataDir),
    })
    logger.info('TSyringe: Using group-scoped storage (AuthorizedGroupFileStorage)')
  } else {
    container.register('Storage', {
      useValue: new FileStorage(dataDir),
    })
    logger.info('TSyringe: Using default storage (FileStorage)')
  }
}

/**
 * Create service container using TSyringe for dependency injection
 */
async function createServiceContainerWithTSyringe(): Promise<ServiceContainer> {
  // Import all service classes and infrastructure
  const {
    HostnameProxyService,
    InMemoryPreviewServerRegistryStore,
    PreviewServerRegistry,
    RegistryTargetResolver,
  } = await import('@moatless/preview-server')

  // Import and resolve all services
  const { AgentService } = await import('./services/agent.service.js')
  const { AuthService } = await import('./features/auth/services/auth.service.js')
  const { ClaudeService } = await import('./agents/claude-code/claude.service.js')
  const { CommandService } = await import('./services/command.service.js')
  const { DockerService } = await import('./services/docker.service.js')
  const { EventService } = await import('./services/event.service.js')
  const { FigmaService } = await import('./services/figma.service.js')
  const { GitHubService } = await import('./services/github.service.js')
  const { McpService } = await import('./services/mcp.service.js')
  const { MessageService } = await import('./services/message.service.js')
  const { NetlifyService } = await import('./services/netlify.service.js')
  const { RepositoryService } = await import('./services/repository.service.js')
  const { ScreenshotService } = await import('./services/screenshot.service.js')
  const { SettingsService } = await import('./services/settings.service.js')
  const { TaskService } = await import('./services/task.service.js')
  const { ToolService } = await import('./services/tool.service.js')
  const { UploadService } = await import('./services/upload.service.js')
  const { WorkspaceService } = await import('./services/workspace.service.js')

  // Create infrastructure clients
  const { createGitHubClient } = await import('./infrastructure/clients/github.client.js')
  const { createAuth0Client } = await import('./infrastructure/clients/auth0.client.js')
  const githubClient = createGitHubClient()
  const auth0Client = createAuth0Client()

  // Register infrastructure clients in TSyringe
  container.register('GitHubClient', { useValue: githubClient })
  container.register('Auth0Client', { useValue: auth0Client })

  // Create preview registry
  const previewRegistry = new PreviewServerRegistry(
    new InMemoryPreviewServerRegistryStore(),
    logger.child({ service: 'preview-registry' }),
  )
  container.register('PreviewServerRegistry', { useValue: previewRegistry })

  // Create OAuth provider map
  const { createGitHubOAuthProvider, createAuth0OAuthProvider } = await import(
    './features/auth/index.js'
  )
  const providers = new Map()
  providers.set('github', createGitHubOAuthProvider(githubClient))
  providers.set('auth0', createAuth0OAuthProvider(auth0Client))
  container.register('OAuthProviders', { useValue: providers })

  // Resolve all services from TSyringe
  const storage = container.resolve<StorageBackend>('Storage')
  const settings = container.resolve(SettingsService)
  const upload = container.resolve(UploadService)
  const repository = container.resolve(RepositoryService)
  const claude = container.resolve(ClaudeService)
  const command = container.resolve(CommandService)
  const docker = container.resolve(DockerService)
  const netlify = container.resolve(NetlifyService)
  const event = container.resolve(EventService)
  const github = container.resolve(GitHubService)
  const figma = container.resolve(FigmaService)
  const auth = container.resolve(AuthService)
  const workspace = container.resolve(WorkspaceService)
  const message = container.resolve(MessageService)
  const agent = container.resolve(AgentService)
  const screenshot = container.resolve(ScreenshotService)
  const task = container.resolve(TaskService)
  const tool = container.resolve(ToolService)
  const mcp = container.resolve(McpService)

  // Initialize hostname proxy service
  const targetResolver = new RegistryTargetResolver(
    previewRegistry,
    logger.child({ service: 'hostname-resolver' }),
  )
  await HostnameProxyService.getInstance(logger, targetResolver)

  // Set additional dependencies via setter methods
  repository.setClaudeService(claude)
  repository.setSettingsService(settings)

  // Set global tool service instance for agent access (backward compatibility)
  const { setToolService } = await import('./services/tool.service.js')
  setToolService(tool)

  // Return service container
  return {
    agent,
    auth,
    claude,
    command,
    docker,
    event,
    figma,
    github,
    mcp,
    message,
    netlify,
    previewRegistry,
    repository,
    screenshot,
    settings,
    storage,
    task,
    tool,
    upload,
    workspace,
  }
}

export async function createApp(): Promise<AppWithServices> {
  const app = new OpenAPIHono<AppBindings>()

  // Request ID middleware (must be first)
  app.use('*', requestId)

  // OpenTelemetry middleware (if enabled via APPLICATIONINSIGHTS_CONNECTION_STRING)
  if (process.env['APPLICATIONINSIGHTS_CONNECTION_STRING']) {
    try {
      const { otel } = await import('@hono/otel')
      app.use('*', otel())
      logger.info('OpenTelemetry middleware enabled - HTTP requests will be traced')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.warn(`Failed to load OpenTelemetry middleware: ${errorMessage}`)
    }
  }

  // Logger middleware
  app.use('*', loggerMiddleware)

  // Security headers
  app.use('*', secureHeaders())

  // CORS
  app.use(
    '*',
    cors({
      origin: (origin) => {
        // In development, allow all localhost origins
        if (process.env['NODE_ENV'] !== 'production' && origin) {
          const url = new URL(origin)
          if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
            return origin
          }
        }

        // In production or for non-localhost origins, use configured origin
        return process.env['CORS_ORIGIN'] || origin || '*'
      },
      credentials: true,
    }),
  )

  // Register TSyringe infrastructure
  await registerStorageBackend()

  // Setup services and inject into context
  const services = await createServiceContainerWithTSyringe()
  app.use('*', async (c, next) => {
    c.set('services', services)
    await next()
  })

  // Authentication middleware
  // Active when AUTH_PROVIDER is set to 'github' or 'auth0'
  const authProvider = process.env['AUTH_PROVIDER']

  if (authProvider) {
    // Validate auth provider
    if (authProvider !== 'github' && authProvider !== 'auth0') {
      throw new Error(
        `Invalid AUTH_PROVIDER: "${authProvider}". Must be 'github' or 'auth0', or leave empty for no authentication.`,
      )
    }

    logger.info(`Authentication enabled with provider: ${authProvider}`)

    app.use('*', auth)
    // Session context middleware - stores session in AsyncLocalStorage for request lifecycle
    // Must be after auth middleware to capture the session set by auth
    app.use('*', sessionContext)
  } else {
    logger.info('Authentication disabled - AUTH_PROVIDER not set')
  }

  // Error handling middleware
  app.onError(errorHandler)

  // Register API routes
  await registerRoutes(app)

  // Health check
  app.get('/health', (c) => {
    return c.json({
      status: 'ok',
      timestamp: new Date().toISOString(),
      version: process.env['npm_package_version'] || '1.0.0',
    })
  })

  // OpenAPI specification endpoint (automatically generated from routes)
  app.doc('/openapi.json', {
    openapi: '3.0.0',
    info: {
      title: 'Moatless API',
      version: '1.0.0',
      description:
        'API for the Moatless application - AI-powered task management and code generation',
    },
    servers: [
      {
        url: process.env['API_URL'] || 'http://localhost:8000',
        description:
          process.env['NODE_ENV'] === 'production' ? 'Production server' : 'Development server',
      },
    ],
  })

  // API documentation UI with Scalar
  app.get(
    '/docs',
    apiReference({
      theme: 'purple',
      url: '/openapi.json',
    }),
  )

  // Serve client build in production
  if (process.env['NODE_ENV'] === 'production') {
    // Serve static files
    const staticPath =
      process.env['STATIC_CLIENT_PATH'] || path.join(__dirname, '../../frontend/dist')
    app.use(
      '*',
      except(
        '/api/*',
        serveStatic({
          root: staticPath,
          getContent: async (path: string) => {
            try {
              const fs = await import('node:fs/promises')
              const buffer = await fs.readFile(path)
              return new Uint8Array(buffer)
            } catch {
              return null
            }
          },
          onFound: (path: string, c) => {
            // Set proper MIME type for source maps
            if (path.endsWith('.js.map')) {
              c.header('Content-Type', 'application/json')
            } else if (path.endsWith('.css.map')) {
              c.header('Content-Type', 'application/json')
            }
          },
        }),
      ),
    )

    // SPA fallback for non-API routes
    app.get('*', async (c) => {
      // Skip API routes
      if (c.req.path.startsWith('/api') || c.req.path.startsWith('/health')) {
        return c.notFound()
      }

      // Serve the actual built index.html
      const fs = await import('node:fs/promises')
      const indexPath = path.join(staticPath, 'index.html')
      try {
        const indexHtml = await fs.readFile(indexPath, 'utf-8')
        return c.html(indexHtml)
      } catch (error) {
        logger.error('Failed to read index.html', error)
        return c.notFound()
      }
    })
  } else {
    // In development mode, return 404 for unknown routes
    app.get('*', (c) => {
      return c.notFound()
    })
  }

  return { app, services }
}
