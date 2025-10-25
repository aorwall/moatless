import type { PreviewServerRegistry } from '@moatless/preview-server'
import { Container } from 'inversify'
import type { OAuthProvider } from '../features/auth/providers/oauth-provider.interface.js'
import { OAuthProvidersSymbol } from '../features/auth/providers/oauth-provider.interface.js'
import type { Auth0Client } from '../infrastructure/clients/auth0.client.js'
import { Auth0ClientSymbol } from '../infrastructure/clients/auth0.client.js'
import type { GitHubClient } from '../infrastructure/clients/github.client.js'
import { GitHubClientSymbol } from '../infrastructure/clients/github.client.js'
import { InfrastructureStorageSymbol } from '../infrastructure/storage/storage.js'
import type { StorageBackend } from '../storage/storage-backend.js'
import { StorageSymbol } from '../storage/storage-backend.js'
import { PreviewServerLifecycleSinkSymbol } from './identifiers.js'

/**
 * Global application container.
 * Import this in routes and services to resolve dependencies.
 */
export let appContainer: Container

/**
 * Initialize the application container with infrastructure and services.
 * Should be called once during application bootstrap.
 *
 * @param infrastructure - Pre-configured infrastructure components
 * @returns Configured InversifyJS container ready for service resolution
 */
export async function initializeContainer(infrastructure: {
  storage: StorageBackend
  githubClient: GitHubClient
  auth0Client: Auth0Client
  previewRegistry: PreviewServerRegistry
  oauthProviders: Map<string, OAuthProvider>
}): Promise<Container> {
  appContainer = createContainer(infrastructure)
  await bindServices(appContainer)

  // Handle special initialization cases
  const { SettingsService } = await import('../services/settings.service.js')
  const settingsService = new SettingsService(null) // null = use default path
  await settingsService.initialize()
  appContainer.bind(SettingsService).toConstantValue(settingsService)

  return appContainer
}

/**
 * Create base container with infrastructure bindings.
 * Internal helper for initializeContainer.
 */
function createContainer(infrastructure: {
  storage: StorageBackend
  githubClient: GitHubClient
  auth0Client: Auth0Client
  previewRegistry: PreviewServerRegistry
  oauthProviders: Map<string, OAuthProvider>
}): Container {
  const container = new Container()

  // Bind infrastructure components (use toConstantValue for pre-configured instances)
  container.bind(StorageSymbol).toConstantValue(infrastructure.storage)
  // Bind same storage instance to infrastructure symbol for new feature modules
  container.bind(InfrastructureStorageSymbol).toConstantValue(infrastructure.storage)
  container.bind(GitHubClientSymbol).toConstantValue(infrastructure.githubClient)
  container.bind(Auth0ClientSymbol).toConstantValue(infrastructure.auth0Client)
  container.bind('PreviewServerRegistry').toConstantValue(infrastructure.previewRegistry)
  container.bind(PreviewServerLifecycleSinkSymbol).toConstantValue(infrastructure.previewRegistry)
  container.bind(OAuthProvidersSymbol).toConstantValue(infrastructure.oauthProviders)

  return container
}

/**
 * Bind all application services to the container.
 * Services are bound with .toSelf().inSingletonScope() pattern.
 *
 * This function is extracted to allow test containers to reuse binding logic.
 */
export async function bindServices(container: Container): Promise<void> {
  // Import all service classes
  const { AgentService } = await import('../services/agent.service.js')
  const { AuthService } = await import('../features/auth/services/auth.service.js')
  const { ClaudeService } = await import('../agents/claude-code/claude.service.js')
  const { CommandService } = await import('../services/command.service.js')
  const { DockerService } = await import('../services/docker.service.js')
  const { EventService } = await import('../features/events/services/event.service.js')
  const { EventsStorage } = await import('../features/events/storage/events.storage.js')
  const { FigmaService } = await import('../services/figma.service.js')
  const { GitHubService } = await import('../services/github.service.js')
  const { McpService } = await import('../services/mcp.service.js')
  const { MessageService } = await import('../services/message.service.js')
  const { NetlifyService } = await import('../services/netlify.service.js')
  const { RepositoryService } = await import(
    '../features/repositories/services/repository.service.js'
  )
  const { RepositoryGitService } = await import(
    '../features/repositories/services/repository-git.service.js'
  )
  const { RepositoryStorage } = await import(
    '../features/repositories/storage/repository.storage.js'
  )
  const { ScreenshotService } = await import('../services/screenshot.service.js')
  const { ScreenshotOrchestrationService } = await import(
    '../services/screenshot-orchestration.service.js'
  )
  const { SessionService } = await import('../features/auth/services/session.service.js')
  const { SessionStorage } = await import('../features/auth/storage/session.storage.js')
  const { TaskService } = await import('../services/task.service.js')
  const { ToolService } = await import('../services/tool.service.js')
  const { UploadService } = await import('../services/upload.service.js')
  const { UserStorage } = await import('../features/auth/storage/user.storage.js')
  const { WorkspaceService } = await import('../services/workspace.service.js')

  // Bind all services with singleton scope
  container.bind(AgentService).toSelf().inSingletonScope()
  container.bind(AuthService).toSelf().inSingletonScope()
  container.bind(ClaudeService).toSelf().inSingletonScope()
  container.bind(CommandService).toSelf().inSingletonScope()
  container.bind(DockerService).toSelf().inSingletonScope()
  container.bind(EventService).toSelf().inSingletonScope()
  container.bind(EventsStorage).toSelf().inSingletonScope()
  container.bind(FigmaService).toSelf().inSingletonScope()
  container.bind(GitHubService).toSelf().inSingletonScope()
  container.bind(McpService).toSelf().inSingletonScope()
  container.bind(MessageService).toSelf().inSingletonScope()
  container.bind(NetlifyService).toSelf().inSingletonScope()
  container.bind(RepositoryService).toSelf().inSingletonScope()
  container.bind(RepositoryGitService).toSelf().inSingletonScope()
  container.bind(RepositoryStorage).toSelf().inSingletonScope()
  container.bind(ScreenshotService).toSelf().inSingletonScope()
  container.bind(ScreenshotOrchestrationService).toSelf().inSingletonScope()
  // SettingsService is bound separately in initializeContainer() after initialization
  container.bind(SessionService).toSelf().inSingletonScope()
  container.bind(SessionStorage).toSelf().inSingletonScope()
  container.bind(TaskService).toSelf().inSingletonScope()
  container.bind(ToolService).toSelf().inSingletonScope()
  container.bind(UploadService).toSelf().inSingletonScope()
  container.bind(UserStorage).toSelf().inSingletonScope()
  container.bind(WorkspaceService).toSelf().inSingletonScope()
}

/**
 * Create storage backend instance based on STORAGE_TYPE env var.
 * Internal helper for initializeAppContainer.
 */
async function createStorageBackend(): Promise<StorageBackend> {
  const storageType = process.env['STORAGE_TYPE']
  const authProvider = process.env['AUTH_PROVIDER']

  // Import storage classes and utilities
  const { getDataDir } = await import('../utils/data-dir.js')
  const { FileStorage, AuthorizedFileStorage, AuthorizedGroupFileStorage } = await import(
    '../storage/index.js'
  )
  const { promises: fs } = await import('node:fs')
  const { logger } = await import('../utils/logger.js')

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
 * Initialize the application container with all infrastructure and services.
 * This is the main entry point for setting up dependency injection.
 * Call this once during application bootstrap.
 */
export async function initializeAppContainer(): Promise<void> {
  // Import infrastructure
  const {
    HostnameProxyService,
    InMemoryPreviewServerRegistryStore,
    PreviewServerRegistry,
    RegistryTargetResolver,
  } = await import('@moatless/preview-server')

  // Create infrastructure dependencies
  const storage = await createStorageBackend()
  const { createGitHubClient } = await import('../infrastructure/clients/github.client.js')
  const { createAuth0Client } = await import('../infrastructure/clients/auth0.client.js')
  const githubClient = createGitHubClient()
  const auth0Client = createAuth0Client()

  // Create preview registry
  const { logger } = await import('../utils/logger.js')
  const previewRegistry = new PreviewServerRegistry(
    new InMemoryPreviewServerRegistryStore(),
    logger.child({ service: 'preview-registry' }),
  )

  // Create OAuth provider map
  const { createGitHubOAuthProvider, createAuth0OAuthProvider } = await import(
    '../features/auth/index.js'
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
    '../features/repositories/services/repository.service.js'
  )
  const { ClaudeService } = await import('../agents/claude-code/claude.service.js')
  const { SettingsService } = await import('../services/settings.service.js')
  const { GitHubService } = await import('../services/github.service.js')

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
