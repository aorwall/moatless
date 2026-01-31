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

  // Bind workspace provisioner based on execution mode
  const executionMode = settingsService.getWorkspaceExecutionMode()
  const { DockerWorkspaceProvisioner, KubernetesWorkspaceProvisioner } = await import(
    '../features/workspace/provisioners/index.js'
  )

  if (executionMode === 'docker') {
    appContainer.bind('WorkspaceProvisioner').to(DockerWorkspaceProvisioner).inSingletonScope()
  } else {
    // Default to Kubernetes for 'kubernetes' mode
    appContainer.bind('WorkspaceProvisioner').to(KubernetesWorkspaceProvisioner).inSingletonScope()
  }

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
  const { FigmaService } = await import('../features/figma/services/figma.service.js')
  const { FigmaStorage } = await import('../features/figma/storage/figma.storage.js')
  const { MessageService } = await import('../features/agents/services/message.service.js')
  const { ClaudeSettingsService } = await import('../features/claude/index.js')

  // Import workspace services BEFORE AgentController since AgentController depends on them
  const { WorkspaceRuntimeRepository } = await import(
    '../features/workspace/db/workspace-runtime.repository.js'
  )
  const { WorkspaceRuntimeManager } = await import(
    '../features/workspace/services/workspace-runtime-manager.service.js'
  )
  const { ProvisionPlanFactory } = await import(
    '../features/workspace/services/provision-plan-factory.service.js'
  )
  // Note: Drivers are not imported here.
  // They are created per-task by provisioners and are not bound to DI container.
  const { KubernetesService } = await import('../services/kubernetes.service.js')

  // Import workspace provisioners
  const { DockerWorkspaceProvisioner, KubernetesWorkspaceProvisioner } = await import(
    '../features/workspace/provisioners/index.js'
  )

  // Import preview feature services
  const { PreviewOrchestratorService } = await import(
    '../features/preview/services/preview-orchestrator.service.js'
  )
  const { PreviewRuntimeService } = await import(
    '../features/preview/services/preview-runtime.service.js'
  )
  // Import agent services AFTER workspace services (they depend on FileGateway and WorkspaceRuntimeManager)
  // Import directly from file (not barrel) to avoid circular module loading
  const { AgentRequestBuilder } = await import(
    '../features/agents/services/agent-request-builder.service.js'
  )
  const { AgentControlService } = await import(
    '../features/agents/services/agent-control.service.js'
  )
  const { AgentRequestService } = await import(
    '../features/agents/services/agent-request.service.js'
  )
  const { TaskIterationService } = await import(
    '../features/tasks/services/task-iteration.service.js'
  )
  const { TaskEventHandler } = await import(
    '../features/tasks/services/task-event-handler.service.js'
  )
  const { PendingResponseService } = await import(
    '../features/tasks/services/pending-response.service.js'
  )
  const { TaskExecutionCoordinator } = await import(
    '../features/tasks/services/task-execution-coordinator.service.js'
  )
  const { TaskExecutionCoordinatorSymbol } = await import('../features/tasks/symbols.js')
  const { TaskCommandGateway } = await import(
    '../features/tasks/services/task-command-gateway.service.js'
  )
  const { TaskService } = await import('../features/tasks/services/task.service.js')
  const { DeploymentService } = await import(
    '../features/deployments/services/deployment.service.js'
  )
  const { NetlifyService } = await import('../features/deployments/services/netlify.service.js')
  const { RepositoryService } = await import(
    '../features/repositories/services/repository.service.js'
  )
  const { SessionService } = await import('../features/auth/services/session.service.js')
  const { GitHubTokenService } = await import('../features/auth/services/github-token.service.js')
  const { GitLabTokenService } = await import('../features/auth/services/gitlab-token.service.js')
  const { GitProviderTokenService } = await import(
    '../features/auth/services/git-provider-token.service.js'
  )
  const { ClaudeTokenService } = await import('../features/claude/index.js')
  const { TaskMessageRepository } = await import('../features/tasks/db/task-message.repository.js')
  const { TaskRepository } = await import('../features/tasks/db/task.repository.js')
  const { FileStorageService } = await import('../features/file-storage/index.js')

  // Import DI symbols
  const { ClaudeServiceSymbol } = await import('../features/claude/index.js')
  const { AgentRequestBuilderSymbol, AgentControlServiceSymbol } = await import(
    '../features/agents/index.js'
  )

  // Bind all services with singleton scope
  container.bind(AuthService).toSelf().inSingletonScope()
  container.bind(ClaudeService).toSelf().inSingletonScope()
  // Bind ClaudeService symbol to break circular dependencies with legacy services
  container.bind(ClaudeServiceSymbol).to(ClaudeService)
  container.bind(CommandService).toSelf().inSingletonScope()
  container.bind(DockerService).toSelf().inSingletonScope()
  container.bind(EventService).toSelf().inSingletonScope()
  container.bind(FigmaService).toSelf().inSingletonScope()
  container.bind(FigmaStorage).toSelf().inSingletonScope()
  container.bind(MessageService).toSelf().inSingletonScope()
  container.bind(ClaudeSettingsService).toSelf().inSingletonScope()
  container.bind(TaskIterationService).toSelf().inSingletonScope()
  container.bind(PendingResponseService).toSelf().inSingletonScope()
  container.bind(TaskExecutionCoordinator).toSelf().inSingletonScope()
  // Bind TaskExecutionCoordinator symbol to break circular dependencies with TaskEventHandler
  container.bind(TaskExecutionCoordinatorSymbol).toService(TaskExecutionCoordinator)
  container.bind(TaskEventHandler).toSelf().inSingletonScope()
  container.bind(TaskCommandGateway).toSelf().inSingletonScope()
  container.bind(TaskService).toSelf().inSingletonScope()
  container.bind(DeploymentService).toSelf().inSingletonScope()
  container.bind(NetlifyService).toSelf().inSingletonScope()
  container.bind(RepositoryService).toSelf().inSingletonScope()
  // SettingsService is bound separately in initializeContainer() after initialization
  container.bind(SessionService).toSelf().inSingletonScope()
  container.bind(GitHubTokenService).toSelf().inSingletonScope()
  container.bind(GitLabTokenService).toSelf().inSingletonScope()
  container.bind(GitProviderTokenService).toSelf().inSingletonScope()
  container.bind(ClaudeTokenService).toSelf().inSingletonScope()
  // Tasks feature repositories (Drizzle-based)
  container.bind(TaskRepository).toSelf().inSingletonScope()
  container.bind(TaskMessageRepository).toSelf().inSingletonScope()
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
  const { PermissionService } = await import('../features/auth/services/permission.service.js')
  container.bind(UserRepository).toSelf().inSingletonScope()
  container.bind(SessionRepository).toSelf().inSingletonScope()
  container.bind(ProviderTokenRepository).toSelf().inSingletonScope()
  container.bind(OAuthStateRepository).toSelf().inSingletonScope()
  container.bind(ApiKeyRepository).toSelf().inSingletonScope()
  container.bind(ApiKeyService).toSelf().inSingletonScope()
  container.bind(PermissionService).toSelf().inSingletonScope()
  // Bind ProviderTokenRepository to string symbol for GitHubTokenService
  container.bind('ProviderTokenRepository').to(ProviderTokenRepository)

  // Import and bind repository repository (Drizzle-based)
  const { RepositoryRepository } = await import(
    '../features/repositories/db/repository.repository.js'
  )
  container.bind(RepositoryRepository).toSelf().inSingletonScope()

  // Import and bind channels feature services
  const { ChannelThreadRepository } = await import(
    '../features/channels/db/channel-thread.repository.js'
  )
  const { SlackChannelMappingRepository } = await import(
    '../features/channels/db/slack-channel-mapping.repository.js'
  )
  const { ThreadTrackerService } = await import(
    '../features/channels/services/thread-tracker.service.js'
  )
  const { UserResolverService } = await import(
    '../features/channels/services/user-resolver.service.js'
  )
  const { EmailInboundService } = await import(
    '../features/channels/email/services/email-inbound.service.js'
  )
  const { EmailOutboundService } = await import(
    '../features/channels/email/services/email-outbound.service.js'
  )
  const { SlackChannelMappingService } = await import(
    '../features/channels/slack/services/slack-channel-mapping.service.js'
  )
  const { SlackEventService } = await import(
    '../features/channels/slack/services/slack-event.service.js'
  )
  const { SlackMessageService } = await import(
    '../features/channels/slack/services/slack-message.service.js'
  )
  const { ChannelNotificationService } = await import(
    '../features/channels/services/channel-notification.service.js'
  )
  container.bind(ChannelThreadRepository).toSelf().inSingletonScope()
  container.bind(SlackChannelMappingRepository).toSelf().inSingletonScope()
  container.bind(ThreadTrackerService).toSelf().inSingletonScope()
  container.bind(UserResolverService).toSelf().inSingletonScope()
  container.bind(EmailInboundService).toSelf().inSingletonScope()
  container.bind(EmailOutboundService).toSelf().inSingletonScope()
  container.bind(SlackChannelMappingService).toSelf().inSingletonScope()
  container.bind(SlackEventService).toSelf().inSingletonScope()
  container.bind(SlackMessageService).toSelf().inSingletonScope()
  container.bind(ChannelNotificationService).toSelf().inSingletonScope()

  // Import and bind tag repository (Drizzle-based)
  const { TagRepository } = await import('../features/tags/db/tag.repository.js')
  container.bind(TagRepository).toSelf().inSingletonScope()

  // Import and bind agents repositories (Drizzle-based)
  const { CommandRepository } = await import('../features/agents/db/command.repository.js')
  container.bind(CommandRepository).toSelf().inSingletonScope()

  // Import and bind plugins feature services
  const { PluginMarketplaceRepository } = await import(
    '../features/plugins/db/plugin-marketplace.repository.js'
  )
  const { PluginService } = await import('../features/plugins/services/plugin.service.js')
  container.bind(PluginMarketplaceRepository).toSelf().inSingletonScope()
  container.bind(PluginService).toSelf().inSingletonScope()

  container.bind(AgentRequestBuilder).toSelf().inSingletonScope()
  // Bind AgentRequestBuilder symbol to break circular dependencies with task services
  container.bind(AgentRequestBuilderSymbol).to(AgentRequestBuilder)
  container.bind(AgentControlService).toSelf().inSingletonScope()
  // Bind AgentControlService symbol to break circular dependencies
  container.bind(AgentControlServiceSymbol).to(AgentControlService)
  container.bind(AgentRequestService).toSelf().inSingletonScope()
  container.bind(WorkspaceRuntimeRepository).toSelf().inSingletonScope()
  container.bind(ProvisionPlanFactory).toSelf().inSingletonScope()
  container.bind(WorkspaceRuntimeManager).toSelf().inSingletonScope()
  // Note: Drivers are NOT bound here.
  // They are created per-task by provisioners (driver factory pattern).
  container.bind(KubernetesService).toSelf().inSingletonScope()

  // Bind workspace provisioners
  // Both are bound to DI so they can be resolved, but only one is bound to 'WorkspaceProvisioner'
  // symbol based on WORKSPACE_EXECUTION_MODE (see initializeContainer)
  container.bind(DockerWorkspaceProvisioner).toSelf().inSingletonScope()
  container.bind(KubernetesWorkspaceProvisioner).toSelf().inSingletonScope()

  // Bind preview feature services
  container.bind(PreviewRuntimeService).toSelf().inSingletonScope()
  container.bind(PreviewOrchestratorService).toSelf().inSingletonScope()

  // Bind metrics services
  const { RuntimeHealthCollector } = await import(
    '../features/metrics/services/runtime-health-collector.service.js'
  )
  container.bind(RuntimeHealthCollector).toSelf().inSingletonScope()

  // Bind workspace inactivity service
  const { WorkspaceInactivityService } = await import(
    '../features/workspace/services/workspace-inactivity.service.js'
  )
  container.bind(WorkspaceInactivityService).toSelf().inSingletonScope()
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

  // Get database configuration from environment (requires DATABASE_URL)
  const dbConfig = getDatabaseConfig()
  const database = createDatabase(dbConfig)
  await database.migrate()

  logger.info('PostgreSQL database initialized')

  // Load and validate channel configuration (email, Slack)
  // This must be done early so the app fails fast if channels are misconfigured
  const { loadChannelsConfig, validateChannelsConfig } = await import(
    '../features/channels/config/channels.config.js'
  )
  const channelsConfig = loadChannelsConfig()
  validateChannelsConfig(channelsConfig)

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

  // Configure and initialize Redis event transport
  const { loadTransportConfig, createEventTransport } = await import(
    '../features/events/config/transport.config.js'
  )
  const { EventService } = await import('../features/events/services/event.service.js')

  const transportConfig = loadTransportConfig()
  const eventService = appContainer.get(EventService)

  // Create event logger adapter
  const eventLogger = {
    debug: (obj: object, msg?: string) => logger.debug(obj, msg),
    info: (obj: object, msg?: string) => logger.info(obj, msg),
    warn: (obj: object, msg?: string) => logger.warn(obj, msg),
    error: (obj: object, msg?: string) => logger.error(obj, msg),
  }

  const transport = await createEventTransport(transportConfig, eventLogger)

  // Attach to EventService
  eventService.setTransport(transport)

  // Backend always consumes from Redis and broadcasts to SSE
  await transport.startListening(async (event) => {
    // Broadcast to SSE clients via EventEmitter
    await eventService.emitEvent(event)
  })

  logger.info({ redisUrl: transportConfig.redisUrl }, 'Redis event transport initialized')

  // Initialize and bind Redis RequestQueue for agent execution requests
  const { RedisRequestQueue } = await import('@moatless/messaging')
  const { RequestQueueSymbol } = await import('../features/agents/symbols.js')
  type AgentRunRequest = import('@moatless/sandbox-schemas').AgentRunRequest

  const requestQueue = new RedisRequestQueue<AgentRunRequest>({
    redisUrl: transportConfig.redisUrl,
    logger: eventLogger,
    consumerGroup: 'agent-request-consumers',
    consumerId: `backend-${process.env['HOSTNAME'] || 'local'}`,
  })
  await requestQueue.initialize()
  appContainer.bind(RequestQueueSymbol).toConstantValue(requestQueue)

  logger.info({ redisUrl: transportConfig.redisUrl }, 'Redis request queue initialized')

  // Start message persistence subscription for both local and remote agents
  const { MessageService } = await import('../features/agents/services/message.service.js')
  const messageService = appContainer.get(MessageService)
  messageService.startEventSubscription()

  // Start channel notification subscription for sending replies to Slack/Email
  const { ChannelNotificationService } = await import(
    '../features/channels/services/channel-notification.service.js'
  )
  const channelNotification = appContainer.get(ChannelNotificationService)
  channelNotification.startEventSubscription()

  // Start runtime health collector when metrics are enabled
  const { metricsEnabled } = await import('../features/metrics/index.js')
  if (metricsEnabled) {
    const { RuntimeHealthCollector } = await import(
      '../features/metrics/services/runtime-health-collector.service.js'
    )
    const healthCollector = appContainer.get(RuntimeHealthCollector)
    const intervalMs = Number(process.env['HEALTH_COLLECTOR_INTERVAL_MS']) || 30000
    healthCollector.start(intervalMs)
    logger.info({ intervalMs }, 'Runtime health collector started')
  }

  // Start workspace inactivity checker
  const { WorkspaceInactivityService } = await import(
    '../features/workspace/services/workspace-inactivity.service.js'
  )
  const inactivityService = appContainer.get(WorkspaceInactivityService)
  inactivityService.start()

  // Start task event handler - handles all task-related events (agent, environment)
  // This consolidates event handling from TaskIterationService and TaskExecutionCoordinator
  const { TaskEventHandler } = await import(
    '../features/tasks/services/task-event-handler.service.js'
  )
  const taskEventHandler = appContainer.get(TaskEventHandler)
  await taskEventHandler.recoverOrphanedIterations()
  taskEventHandler.startEventSubscription()

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

  // Seed default template repositories on first startup
  const { seedDefaultRepositories } = await import(
    '../features/repositories/utils/seed-repositories.js'
  )
  await seedDefaultRepositories(appContainer)

  logger.info('Application container initialized successfully')
}
