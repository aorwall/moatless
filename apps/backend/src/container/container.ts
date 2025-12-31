import { Container } from 'inversify'
import type { OAuthProvider } from '../features/auth/providers/oauth-provider.interface'
import { OAuthProvidersSymbol } from '../features/auth/providers/oauth-provider.interface'
import type { Auth0Client } from '../infrastructure/clients/auth0.client'
import { Auth0ClientSymbol } from '../infrastructure/clients/auth0.client'
import type { GitHubClient } from '../infrastructure/clients/github.client'
import { GitHubClientSymbol } from '../infrastructure/clients/github.client'
import type { GitHubProviderClient } from '../infrastructure/clients/github-provider.client'
import { GitHubProviderClientSymbol } from '../infrastructure/clients/github-provider.client'
import type { GitLabClient } from '../infrastructure/clients/gitlab.client'
import { GitLabClientSymbol } from '../infrastructure/clients/gitlab.client'
import type { GitLabProviderClient } from '../infrastructure/clients/gitlab-provider.client'
import { GitLabProviderClientSymbol } from '../infrastructure/clients/gitlab-provider.client'
import type { Database } from '../infrastructure/database/database'
import { DatabaseSymbol } from '../infrastructure/database/database'
import type { Storage } from '../infrastructure/storage/storage'
import { InfrastructureStorageSymbol } from '../infrastructure/storage/storage'

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
  storage: Storage
  database: Database
  githubClient: GitHubClient
  githubProviderClient: GitHubProviderClient
  gitlabClient: GitLabClient
  gitlabProviderClient: GitLabProviderClient
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
  storage: Storage
  database: Database
  githubClient: GitHubClient
  githubProviderClient: GitHubProviderClient
  gitlabClient: GitLabClient
  gitlabProviderClient: GitLabProviderClient
  auth0Client: Auth0Client
  oauthProviders: Map<string, OAuthProvider>
}): Container {
  const container = new Container()

  // Bind infrastructure components (use toConstantValue for pre-configured instances)
  container.bind(InfrastructureStorageSymbol).toConstantValue(infrastructure.storage)
  // Bind database instance for Drizzle-based repositories
  container.bind(DatabaseSymbol).toConstantValue(infrastructure.database)
  container.bind(GitHubClientSymbol).toConstantValue(infrastructure.githubClient)
  container.bind(GitHubProviderClientSymbol).toConstantValue(infrastructure.githubProviderClient)
  container.bind(GitLabClientSymbol).toConstantValue(infrastructure.gitlabClient)
  container.bind(GitLabProviderClientSymbol).toConstantValue(infrastructure.gitlabProviderClient)
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
  const { ClaudeService } = await import('../features/claude/index.js')
  const { CommandService } = await import('../features/agents/services/command.service.js')
  const { DockerService } = await import('../services/docker.service.js')
  const { EventService } = await import('../features/events/services/event.service.js')
  const { EventPollerService } = await import('../features/events/services/event-poller.service.js')
  const { EventsStorage } = await import('../features/events/storage/events.storage.js')
  const { EventRepository } = await import('../features/events/db/event.repository.js')
  const { FigmaService } = await import('../features/figma/services/figma.service.js')
  const { FigmaStorage } = await import('../features/figma/storage/figma.storage.js')
  const { McpService } = await import('../features/agents/services/mcp.service.js')
  const { MessageService } = await import('../features/agents/services/message.service.js')
  const { ClaudeSettingsService } = await import('../features/claude/index.js')
  const { McpSettingsService } = await import('../features/agents/services/mcp-settings.service.js')

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
  // Note: LocalEnvironmentDriver and KubernetesEnvironmentDriver are not imported here.
  // They are created per-task by FileGateway and are not bound to DI container.
  const { RemoteWorkerEnvironment } = await import(
    '../features/workspace/drivers/remote-worker-environment.js'
  )
  const { KubernetesService } = await import('../services/kubernetes.service.js')

  // Import preview feature services
  const { PreviewOrchestratorService } = await import(
    '../features/preview/services/preview-orchestrator.service.js'
  )
  const { PreviewRuntimeService } = await import(
    '../features/preview/services/preview-runtime.service.js'
  )
  const { PlaywrightInteractionService } = await import(
    '../features/preview/services/playwright-interaction.service.js'
  )

  // Import AgentController AFTER workspace services (it depends on FileGateway and WorkspaceRuntimeManager)
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
  const { NetlifyService } = await import('../features/deployments/services/netlify.service.js')
  const { RepositoryService } = await import(
    '../features/repositories/services/repository.service.js'
  )
  const { RepositoryGitService } = await import(
    '../features/repositories/services/repository-git.service.js'
  )
  const { PlaywrightScreenshotService } = await import(
    '../features/preview/services/playwright-screenshot.service.js'
  )
  const { SessionService } = await import('../features/auth/services/session.service.js')
  const { GitHubTokenService } = await import('../features/auth/services/github-token.service.js')
  const { GitLabTokenService } = await import('../features/auth/services/gitlab-token.service.js')
  const { GitProviderTokenService } = await import(
    '../features/auth/services/git-provider-token.service.js'
  )
  const { ClaudeTokenService } = await import('../features/claude/index.js')
  const { WorkerTokenService } = await import('../features/auth/services/worker-token.service.js')
  const { TaskMessageRepository } = await import('../features/tasks/db/task-message.repository.js')
  const { TaskRepository } = await import('../features/tasks/db/task.repository.js')
  const { ToolService } = await import('../features/agents/services/tool.service.js')
  const { FileStorageService } = await import('../features/file-storage/index.js')

  // Import DI symbols
  const { ClaudeServiceSymbol } = await import('../features/claude/index.js')
  const { AgentControllerSymbol } = await import('../features/agents/index.js')

  // Bind all services with singleton scope
  container.bind(AuthService).toSelf().inSingletonScope()
  container.bind(ClaudeService).toSelf().inSingletonScope()
  // Bind ClaudeService symbol to break circular dependencies with legacy services
  container.bind(ClaudeServiceSymbol).to(ClaudeService)
  container.bind(CommandService).toSelf().inSingletonScope()
  container.bind(DockerService).toSelf().inSingletonScope()
  container.bind(EventService).toSelf().inSingletonScope()
  container.bind(EventPollerService).toSelf().inSingletonScope()
  container.bind(EventsStorage).toSelf().inSingletonScope()
  container.bind(EventRepository).toSelf().inSingletonScope()
  container.bind(FigmaService).toSelf().inSingletonScope()
  container.bind(FigmaStorage).toSelf().inSingletonScope()
  container.bind(McpService).toSelf().inSingletonScope()
  container.bind(MessageService).toSelf().inSingletonScope()
  container.bind(ClaudeSettingsService).toSelf().inSingletonScope()
  container.bind(McpSettingsService).toSelf().inSingletonScope()
  container.bind(TaskIterationService).toSelf().inSingletonScope()
  container.bind(TaskExecutionCoordinator).toSelf().inSingletonScope()
  container.bind(TaskEventProjector).toSelf().inSingletonScope()
  container.bind(TaskCommandGateway).toSelf().inSingletonScope()
  container.bind(TaskService).toSelf().inSingletonScope()
  container.bind(NetlifyService).toSelf().inSingletonScope()
  container.bind(RepositoryService).toSelf().inSingletonScope()
  container.bind(RepositoryGitService).toSelf().inSingletonScope()
  container.bind(PlaywrightScreenshotService).toSelf().inSingletonScope()
  // SettingsService is bound separately in initializeContainer() after initialization
  container.bind(SessionService).toSelf().inSingletonScope()
  container.bind(GitHubTokenService).toSelf().inSingletonScope()
  container.bind(GitLabTokenService).toSelf().inSingletonScope()
  container.bind(GitProviderTokenService).toSelf().inSingletonScope()
  container.bind(ClaudeTokenService).toSelf().inSingletonScope()
  container.bind(WorkerTokenService).toSelf().inSingletonScope()
  // Tasks feature repositories (Drizzle-based)
  container.bind(TaskRepository).toSelf().inSingletonScope()
  container.bind(TaskMessageRepository).toSelf().inSingletonScope()
  container.bind(ToolService).toSelf().inSingletonScope()
  container.bind(FileStorageService).toSelf().inSingletonScope()

  // Import and bind auth repositories (Drizzle-based)
  const { UserRepository } = await import('../features/auth/db/user.repository.js')
  const { SessionRepository } = await import('../features/auth/db/session.repository.js')
  const { ProviderTokenRepository } = await import(
    '../features/auth/db/provider-token.repository.js'
  )
  const { OAuthStateRepository } = await import('../features/auth/db/oauth-state.repository.js')
  const { ApiKeyRepository } = await import('../features/auth/db/api-key.repository.js')
  const { ApiKeyService } = await import('../features/auth/services/api-key.service.js')
  container.bind(UserRepository).toSelf().inSingletonScope()
  container.bind(SessionRepository).toSelf().inSingletonScope()
  container.bind(ProviderTokenRepository).toSelf().inSingletonScope()
  container.bind(OAuthStateRepository).toSelf().inSingletonScope()
  container.bind(ApiKeyRepository).toSelf().inSingletonScope()
  container.bind(ApiKeyService).toSelf().inSingletonScope()
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
  // Note: LocalEnvironmentDriver and KubernetesEnvironmentDriver are NOT bound here.
  // They are created per-task by FileGateway (driver factory pattern).
  container.bind(KubernetesService).toSelf().inSingletonScope()
  container.bind(RemoteWorkerEnvironment).toSelf().inSingletonScope()

  // Bind preview feature services
  container.bind(PreviewRuntimeService).toSelf().inSingletonScope()
  container.bind(PreviewOrchestratorService).toSelf().inSingletonScope()
  container.bind(PlaywrightInteractionService).toSelf().inSingletonScope()
}

/**
 * Initialize the application container with all infrastructure and services.
 * This is the main entry point for setting up dependency injection.
 * Call this once during application bootstrap.
 */
export async function initializeAppContainer(): Promise<void> {
  // Create infrastructure dependencies
  const { FileStorage } = await import('../infrastructure/storage/file-storage.js')
  const { getDataDir } = await import('../utils/data-dir.js')
  const { promises: fs } = await import('node:fs')

  const dataDir = getDataDir()
  await fs.mkdir(dataDir, { recursive: true })
  const storage = new FileStorage(dataDir)
  const { createGitHubClient } = await import('../infrastructure/clients/github.client.js')
  const { createGitHubProviderClient } = await import(
    '../infrastructure/clients/github-provider.client.js'
  )
  const { createGitLabClient } = await import('../infrastructure/clients/gitlab.client.js')
  const { createGitLabProviderClient } = await import(
    '../infrastructure/clients/gitlab-provider.client.js'
  )
  const { createAuth0Client } = await import('../infrastructure/clients/auth0.client.js')
  const githubClient = createGitHubClient()
  const githubProviderClient = createGitHubProviderClient()
  const gitlabClient = createGitLabClient()
  const gitlabProviderClient = createGitLabProviderClient()
  const auth0Client = createAuth0Client()

  // Create database and run migrations
  const { logger } = await import('../utils/logger.js')
  const { createDatabase, getDatabaseConfig } = await import(
    '../infrastructure/database/factory.js'
  )
  const { join } = await import('node:path')

  // Get database configuration from environment
  const dbConfig = getDatabaseConfig()

  // For SQLite, use data directory path if not specified
  if (dbConfig.provider === 'sqlite' && !dbConfig.sqlitePath) {
    dbConfig.sqlitePath = join(dataDir, 'moatless.db')
  }

  const database = createDatabase(dbConfig)
  await database.migrate()

  logger.info({ provider: dbConfig.provider }, 'Database initialized')

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
    githubProviderClient,
    gitlabClient,
    gitlabProviderClient,
    auth0Client,
    oauthProviders,
  })

  // Import service classes for post-initialization setup
  const { RepositoryService } = await import(
    '../features/repositories/services/repository.service.js'
  )
  const { ClaudeService } = await import('../features/claude/index.js')
  const { SettingsService } = await import('../services/settings.service.js')
  const { GitProviderTokenService } = await import(
    '../features/auth/services/git-provider-token.service.js'
  )
  const { TaskEventProjector } = await import(
    '../features/tasks/services/task-event-projector.service.js'
  )

  // Set additional dependencies via setter methods (can't use DI for circular deps)
  const repository = appContainer.get(RepositoryService)
  const claude = appContainer.get(ClaudeService)
  const settings = appContainer.get(SettingsService)
  const gitProviderTokenService = appContainer.get(GitProviderTokenService)

  repository.setClaudeService(claude)
  repository.setSettingsService(settings)
  repository.setGitProviderTokenService(gitProviderTokenService)

  // Subscribe TaskEventProjector to event stream for reactive task updates
  const taskEventProjector = appContainer.get(TaskEventProjector)
  taskEventProjector.subscribe()

  // Start event poller for database fallback events (PostgreSQL only)
  const { EventPollerService } = await import('../features/events/services/event-poller.service.js')
  const eventPoller = appContainer.get(EventPollerService)
  eventPoller.start()

  // Recover orphaned active iterations from previous server runs
  const { TaskIterationService } = await import(
    '../features/tasks/services/task-iteration.service.js'
  )
  const taskIterationService = appContainer.get(TaskIterationService)
  await taskIterationService.recoverOrphanedIterations()

  // Initialize hostname-based proxy if enabled
  const { HostnameProxyService } = await import('../features/preview/index.js')
  const { RegistryTargetResolver } = await import('../features/preview/index.js')
  const { WorkspaceRuntimeManager } = await import(
    '../features/workspace/services/workspace-runtime-manager.service.js'
  )
  const { KubernetesService } = await import('../services/kubernetes.service.js')
  const workspaceRuntimeManager = appContainer.get(WorkspaceRuntimeManager)
  // Pass KubernetesService to resolver for direct pod query fallback
  const kubernetesService = appContainer.get(KubernetesService)
  const targetResolver = new RegistryTargetResolver(workspaceRuntimeManager, kubernetesService)
  const hostnameProxy = await HostnameProxyService.getInstance(targetResolver)
  if (hostnameProxy) {
    logger.info('Hostname-based proxy initialized and running')
  }

  logger.info('Application container initialized successfully')
}
