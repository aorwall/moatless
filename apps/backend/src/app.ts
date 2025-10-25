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
import { initializeContainer } from './container/container.js'
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

export type AppBindings = {
  Variables: {
    requestId: string
    session?: UserSession
  }
}

export interface AppWithServices {
  app: OpenAPIHono<AppBindings>
}

/**
 * Create storage backend instance based on STORAGE_TYPE env var
 */
async function createStorageBackend(): Promise<StorageBackend> {
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

  // Create storage with conditional logic based on STORAGE_TYPE
  if (storageType === 'user' && authProvider) {
    logger.info('InversifyJS: Using user-scoped storage (AuthorizedFileStorage)')
    return new AuthorizedFileStorage(dataDir)
  }

  if (storageType === 'group' && authProvider) {
    logger.info('InversifyJS: Using group-scoped storage (AuthorizedGroupFileStorage)')
    return new AuthorizedGroupFileStorage(dataDir)
  }

  logger.info('InversifyJS: Using default storage (FileStorage)')
  return new FileStorage(dataDir)
}

/**
 * Initialize InversifyJS container with infrastructure and services.
 * The container is exported globally and can be imported directly in routes.
 */
async function initializeAppContainer(): Promise<void> {
  // Import infrastructure
  const {
    HostnameProxyService,
    InMemoryPreviewServerRegistryStore,
    PreviewServerRegistry,
    RegistryTargetResolver,
  } = await import('@moatless/preview-server')

  // Create infrastructure dependencies
  const storage = await createStorageBackend()
  const { createGitHubClient } = await import('./infrastructure/clients/github.client.js')
  const { createAuth0Client } = await import('./infrastructure/clients/auth0.client.js')
  const githubClient = createGitHubClient()
  const auth0Client = createAuth0Client()

  // Create preview registry
  const previewRegistry = new PreviewServerRegistry(
    new InMemoryPreviewServerRegistryStore(),
    logger.child({ service: 'preview-registry' }),
  )

  // Create OAuth provider map
  const { createGitHubOAuthProvider, createAuth0OAuthProvider } = await import(
    './features/auth/index.js'
  )
  const oauthProviders = new Map()
  oauthProviders.set('github', createGitHubOAuthProvider(githubClient))
  oauthProviders.set('auth0', createAuth0OAuthProvider(auth0Client))

  // Initialize InversifyJS container with infrastructure and services
  await initializeContainer({
    storage,
    githubClient,
    auth0Client,
    previewRegistry,
    oauthProviders,
  })

  // Import service classes for post-initialization setup
  const { RepositoryService } = await import(
    './features/repositories/services/repository.service.js'
  )
  const { ClaudeService } = await import('./agents/claude-code/claude.service.js')
  const { SettingsService } = await import('./services/settings.service.js')
  const { GitHubService } = await import('./services/github.service.js')
  const { appContainer } = await import('./container/container.js')

  // Set additional dependencies via setter methods (can't use DI for circular deps)
  const repository = appContainer.get(RepositoryService)
  const claude = appContainer.get(ClaudeService)
  const settings = appContainer.get(SettingsService)
  const github = appContainer.get(GitHubService)

  repository.setClaudeService(claude)
  repository.setSettingsService(settings)
  repository.setGitHubService(github)

  // Initialize hostname proxy service
  const targetResolver = new RegistryTargetResolver(
    previewRegistry,
    logger.child({ service: 'hostname-resolver' }),
  )
  await HostnameProxyService.getInstance(logger, targetResolver)
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

  // Initialize InversifyJS container with all services
  await initializeAppContainer()

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

  return { app }
}
