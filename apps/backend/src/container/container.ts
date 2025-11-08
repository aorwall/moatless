import { Container } from 'inversify'
import type { OAuthProvider } from '../features/auth/providers/oauth-provider.interface'
import { OAuthProvidersSymbol } from '../features/auth/providers/oauth-provider.interface'
import type { Auth0Client } from '../infrastructure/clients/auth0.client'
import { Auth0ClientSymbol } from '../infrastructure/clients/auth0.client'
import type { GitHubClient } from '../infrastructure/clients/github.client'
import { GitHubClientSymbol } from '../infrastructure/clients/github.client'
import type { Database } from '../infrastructure/database/database'
import { DatabaseSymbol } from '../infrastructure/database/database'
import { InfrastructureStorageSymbol } from '../infrastructure/storage/storage'
import type { StorageBackend } from '../storage/storage-backend'
import { StorageSymbol } from '../storage/storage-backend'

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
  database: Database
  githubClient: GitHubClient
  auth0Client: Auth0Client
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
  database: Database
  githubClient: GitHubClient
  auth0Client: Auth0Client
  oauthProviders: Map<string, OAuthProvider>
}): Container {
  const container = new Container()

  // Bind infrastructure components (use toConstantValue for pre-configured instances)
  container.bind(StorageSymbol).toConstantValue(infrastructure.storage)
  // Bind same storage instance to infrastructure symbol for new feature modules
  container.bind(InfrastructureStorageSymbol).toConstantValue(infrastructure.storage)
  // Bind database instance for Drizzle-based repositories
  container.bind(DatabaseSymbol).toConstantValue(infrastructure.database)
  container.bind(GitHubClientSymbol).toConstantValue(infrastructure.githubClient)
  container.bind(Auth0ClientSymbol).toConstantValue(infrastructure.auth0Client)
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
  const { AuthService } = await import('../features/auth/services/auth.service.js')
  const { ClaudeService } = await import('../features/agents/index.js')
  const { CommandService } = await import('../features/agents/services/command.service.js')
  const { DockerService } = await import('../services/docker.service.js')
  const { EventService } = await import('../features/events/services/event.service.js')
  const { EventsStorage } = await import('../features/events/storage/events.storage.js')
  const { FigmaService } = await import('../features/figma/services/figma.service.js')
  const { FigmaStorage } = await import('../features/figma/storage/figma.storage.js')
  const { McpService } = await import('../services/mcp.service.js')
  const { MessageService } = await import('../features/agents/services/message.service.js')

  // Import workspace services BEFORE AgentController since AgentController depends on them
  const { WorkspaceRuntimeRepository } = await import(
    '../features/workspace/db/workspace-runtime.repository.js'
  )
  const { FileGateway } = await import('../features/workspace/services/file-gateway.service.js')
  const { WorkspaceCoordinator } = await import(
    '../features/workspace/services/workspace-coordinator.service.js'
  )
  const { WorkspaceMaterializer } = await import(
    '../features/workspace/services/workspace-materializer.service.js'
  )
  const { WorkspaceRuntimeManager } = await import(
    '../features/workspace/services/workspace-runtime-manager.service.js'
  )
  const { LocalEnvironmentDriver } = await import(
    '../features/workspace/drivers/local-environment-driver.js'
  )
  const { RemoteWorkerEnvironment } = await import(
    '../features/workspace/drivers/remote-worker-environment.js'
  )

  // Import preview feature services
  const { PreviewOrchestratorService } = await import(
    '../features/preview/services/preview-orchestrator.service.js'
  )
  const { PreviewRuntimeService } = await import(
    '../features/preview/services/preview-runtime.service.js'
  )

  // Import AgentController AFTER workspace services (it depends on LocalEnvironmentDriver and WorkspaceRuntimeManager)
  // Import directly from file (not barrel) to avoid circular module loading
  const { AgentController } = await import(
    '../features/agents/services/agent-controller.service.js'
  )
  const { TaskIterationService } = await import(
    '../features/tasks/services/task-iteration.service.js'
  )
  const { TaskExecutionCoordinator } = await import(
    '../features/tasks/services/task-execution-coordinator.service.js'
  )
  const { TaskEventProjector } = await import(
    '../features/tasks/services/task-event-projector.service.js'
  )
  const { TaskCommandGateway } = await import(
    '../features/tasks/services/task-command-gateway.service.js'
  )
  const { TaskService } = await import('../features/tasks/services/task.service.js')
  const { NetlifyService } = await import('../services/netlify.service.js')
  const { RepositoryService } = await import(
    '../features/repositories/services/repository.service.js'
  )
  const { RepositoryGitService } = await import(
    '../features/repositories/services/repository-git.service.js'
  )
  const { ScreenshotService } = await import('../services/screenshot.service.js')
  const { ScreenshotOrchestrationService } = await import(
    '../services/screenshot-orchestration.service.js'
  )
  const { SessionService } = await import('../features/auth/services/session.service.js')
  const { GitHubTokenService } = await import('../features/auth/services/github-token.service.js')
  const { TaskMessageRepository } = await import('../features/tasks/db/task-message.repository.js')
  const { TaskRepository } = await import('../features/tasks/db/task.repository.js')
  const { ToolService } = await import('../services/tool.service.js')
  const { UploadService } = await import('../services/upload.service.js')

  // Import DI symbols
  const { ClaudeServiceSymbol, AgentControllerSymbol } = await import('../features/agents/index.js')

  // Bind all services with singleton scope
  container.bind(AuthService).toSelf().inSingletonScope()
  container.bind(ClaudeService).toSelf().inSingletonScope()
  // Bind ClaudeService symbol to break circular dependencies with legacy services
  container.bind(ClaudeServiceSymbol).to(ClaudeService)
  container.bind(CommandService).toSelf().inSingletonScope()
  container.bind(DockerService).toSelf().inSingletonScope()
  container.bind(EventService).toSelf().inSingletonScope()
  container.bind(EventsStorage).toSelf().inSingletonScope()
  container.bind(FigmaService).toSelf().inSingletonScope()
  container.bind(FigmaStorage).toSelf().inSingletonScope()
  container.bind(McpService).toSelf().inSingletonScope()
  container.bind(MessageService).toSelf().inSingletonScope()
  container.bind(TaskIterationService).toSelf().inSingletonScope()
  container.bind(TaskExecutionCoordinator).toSelf().inSingletonScope()
  container.bind(TaskEventProjector).toSelf().inSingletonScope()
  container.bind(TaskCommandGateway).toSelf().inSingletonScope()
  container.bind(TaskService).toSelf().inSingletonScope()
  container.bind(NetlifyService).toSelf().inSingletonScope()
  container.bind(RepositoryService).toSelf().inSingletonScope()
  container.bind(RepositoryGitService).toSelf().inSingletonScope()
  container.bind(ScreenshotService).toSelf().inSingletonScope()
  container.bind(ScreenshotOrchestrationService).toSelf().inSingletonScope()
  // SettingsService is bound separately in initializeContainer() after initialization
  container.bind(SessionService).toSelf().inSingletonScope()
  container.bind(GitHubTokenService).toSelf().inSingletonScope()
  // Tasks feature repositories (Drizzle-based)
  container.bind(TaskRepository).toSelf().inSingletonScope()
  container.bind(TaskMessageRepository).toSelf().inSingletonScope()
  container.bind(ToolService).toSelf().inSingletonScope()
  container.bind(UploadService).toSelf().inSingletonScope()

  // Import and bind auth repositories (Drizzle-based)
  const { UserRepository } = await import('../features/auth/db/user.repository.js')
  const { SessionRepository } = await import('../features/auth/db/session.repository.js')
  const { ProviderTokenRepository } = await import(
    '../features/auth/db/provider-token.repository.js'
  )
  const { OAuthStateRepository } = await import('../features/auth/db/oauth-state.repository.js')
  container.bind(UserRepository).toSelf().inSingletonScope()
  container.bind(SessionRepository).toSelf().inSingletonScope()
  container.bind(ProviderTokenRepository).toSelf().inSingletonScope()
  container.bind(OAuthStateRepository).toSelf().inSingletonScope()
  // Bind ProviderTokenRepository to string symbol for GitHubTokenService
  container.bind('ProviderTokenRepository').to(ProviderTokenRepository)

  // Import and bind repository repository (Drizzle-based)
  const { RepositoryRepository } = await import(
    '../features/repositories/db/repository.repository.js'
  )
  container.bind(RepositoryRepository).toSelf().inSingletonScope()

  // Import and bind agents repositories (Drizzle-based)
  const { CommandRepository } = await import('../features/agents/db/command.repository.js')
  container.bind(CommandRepository).toSelf().inSingletonScope()

  container.bind(AgentController).toSelf().inSingletonScope()
  // Bind AgentController symbol to break circular dependencies with task services
  container.bind(AgentControllerSymbol).to(AgentController)
  container.bind(FileGateway).toSelf().inSingletonScope()
  container.bind(WorkspaceRuntimeRepository).toSelf().inSingletonScope()
  container.bind(WorkspaceMaterializer).toSelf().inSingletonScope()
  container.bind(WorkspaceRuntimeManager).toSelf().inSingletonScope()
  container.bind(WorkspaceCoordinator).toSelf().inSingletonScope()
  container.bind(LocalEnvironmentDriver).toSelf().inSingletonScope()
  container.bind(RemoteWorkerEnvironment).toSelf().inSingletonScope()

  // Bind preview feature services
  container.bind(PreviewRuntimeService).toSelf().inSingletonScope()
  container.bind(PreviewOrchestratorService).toSelf().inSingletonScope()
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
  // Create infrastructure dependencies
  const storage = await createStorageBackend()
  const { createGitHubClient } = await import('../infrastructure/clients/github.client.js')
  const { createAuth0Client } = await import('../infrastructure/clients/auth0.client.js')
  const githubClient = createGitHubClient()
  const auth0Client = createAuth0Client()

  // Create database and run migrations
  const { logger } = await import('../utils/logger.js')
  const { createDrizzleDatabase } = await import('../infrastructure/database/drizzle.js')
  const { getDataDir } = await import('../utils/data-dir.js')
  const { promises: fs } = await import('node:fs')
  const { join } = await import('node:path')

  const dataDir = getDataDir()
  await fs.mkdir(dataDir, { recursive: true })
  const dbPath = join(dataDir, 'moatless.db')

  const database = createDrizzleDatabase(dbPath, logger)
  await database.migrate()
  logger.info('Database migrations completed')

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
    database,
    githubClient,
    auth0Client,
    oauthProviders,
  })

  // Import service classes for post-initialization setup
  const { RepositoryService } = await import(
    '../features/repositories/services/repository.service.js'
  )
  const { ClaudeService } = await import('../features/agents/index.js')
  const { SettingsService } = await import('../services/settings.service.js')
  const { GitHubTokenService } = await import('../features/auth/services/github-token.service.js')
  const { TaskEventProjector } = await import(
    '../features/tasks/services/task-event-projector.service.js'
  )

  // Set additional dependencies via setter methods (can't use DI for circular deps)
  const repository = appContainer.get(RepositoryService)
  const claude = appContainer.get(ClaudeService)
  const settings = appContainer.get(SettingsService)
  const githubTokenService = appContainer.get(GitHubTokenService)

  repository.setClaudeService(claude)
  repository.setSettingsService(settings)
  repository.setGitHubTokenService(githubTokenService)

  // Subscribe TaskEventProjector to event stream for reactive task updates
  const taskEventProjector = appContainer.get(TaskEventProjector)
  taskEventProjector.subscribe()

  // Initialize hostname-based proxy if enabled
  const { HostnameProxyService } = await import('../features/preview/index.js')
  const { RegistryTargetResolver } = await import('../features/preview/index.js')
  const { WorkspaceRuntimeManager } = await import(
    '../features/workspace/services/workspace-runtime-manager.service.js'
  )
  const workspaceRuntimeManager = appContainer.get(WorkspaceRuntimeManager)
  const targetResolver = new RegistryTargetResolver(workspaceRuntimeManager)
  const hostnameProxy = await HostnameProxyService.getInstance(targetResolver)
  if (hostnameProxy) {
    logger.info('Hostname-based proxy initialized and running')
  }

  logger.info('Application container initialized successfully')
}
