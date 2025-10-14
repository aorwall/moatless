import type { PreviewServerRegistry } from '@moatless/preview-server'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import type { StorageBackend } from '../storage/index.js'
import type { AgentService } from './agent.service.js'
import type { AuthService } from './auth.service.js'
import type { CommandService } from './command.service.js'
import type { DockerService } from './docker.service.js'
import type { EventService } from './event.service.js'
import type { FigmaService } from './figma.service.js'
import type { GitHubService } from './github.service.js'
import type { McpService } from './mcp.service.js'
import type { MessageService } from './message.service.js'
import type { NetlifyService } from './netlify.service.js'
import type { RepositoryService } from './repository.service.js'
import type { ScreenshotService } from './screenshot.service.js'
import type { SettingsService } from './settings.service.js'
import type { TaskService } from './task.service.js'
import type { ToolService } from './tool.service.js'
import type { UploadService } from './upload.service.js'
import type { WorkspaceService } from './workspace.service.js'

/**
 * Service container interface containing all application services.
 */
export interface ServiceContainer {
  agent: AgentService
  auth: AuthService
  claude: ClaudeService
  command: CommandService
  docker: DockerService
  event: EventService
  figma: FigmaService
  github: GitHubService
  mcp: McpService
  message: MessageService
  netlify: NetlifyService
  previewRegistry: PreviewServerRegistry
  repository: RepositoryService
  screenshot: ScreenshotService
  settings: SettingsService
  storage: StorageBackend
  task: TaskService
  tool: ToolService
  upload: UploadService
  workspace: WorkspaceService
}

/**
 * Creates a fully initialized service container with all dependencies properly wired.
 * This is the main entry point for dependency injection in the application.
 */
export async function createServiceContainer(): Promise<ServiceContainer> {
  const {
    HostnameProxyService,
    InMemoryPreviewServerRegistryStore,
    PreviewServerRegistry,
    RegistryTargetResolver,
  } = await import('@moatless/preview-server')
  const { logger } = await import('../utils/logger.js')
  // Import all service factories
  const { createStorageService } = await import('../storage/index.js')
  const { getGitHubService } = await import('./github.service.js')
  const { createAuthService } = await import('./auth.service.js')
  const { createSettingsService } = await import('./settings.service.js')
  const { createRepositoryService } = await import('./repository.service.js')
  const { createClaudeService } = await import('../agents/claude-code/claude.service.js')
  const { createAgentService } = await import('./agent.service.js')
  const { createCommandService } = await import('./command.service.js')
  const { createDockerService } = await import('./docker.service.js')
  const { createEventService } = await import('./event.service.js')
  const { getFigmaService } = await import('./figma.service.js')
  const { createMcpService } = await import('./mcp.service.js')
  const { createMessageService } = await import('./message.service.js')
  const { createNetlifyService } = await import('./netlify.service.js')
  const { createScreenshotService } = await import('./screenshot.service.js')
  const { createTaskService } = await import('./task.service.js')
  const { createToolService } = await import('./tool.service.js')
  const { createUploadService } = await import('./upload.service.js')
  const { createWorkspaceService } = await import('./workspace.service.js')

  // Create container object that will be populated
  const container: Partial<ServiceContainer> = {}

  container.previewRegistry = new PreviewServerRegistry(
    new InMemoryPreviewServerRegistryStore(),
    logger.child({ service: 'preview-registry' }),
  )

  // Create services in dependency order
  // 1. Core services with no dependencies

  // Select storage backend based on STORAGE_TYPE environment variable
  // - 'user': User-scoped storage (requires GITHUB_OAUTH_ENABLED=true)
  // - 'group': Group-scoped storage (requires GITHUB_OAUTH_ENABLED=true)
  // - 'none' or unset: No access control (default)
  const storageType = process.env['STORAGE_TYPE']
  const { getDataDir } = await import('../utils/data-dir.js')
  const dataDir = getDataDir()

  if (storageType === 'user' && process.env['GITHUB_OAUTH_ENABLED'] === 'true') {
    const { createAuthorizedStorageService } = await import('../storage/index.js')
    container.storage = await createAuthorizedStorageService(dataDir)
    logger.info('Using user-scoped storage (AuthorizedFileStorage)')
  } else if (storageType === 'group' && process.env['GITHUB_OAUTH_ENABLED'] === 'true') {
    const { createAuthorizedGroupStorageService } = await import('../storage/index.js')
    container.storage = await createAuthorizedGroupStorageService(dataDir)
    logger.info('Using group-scoped storage (AuthorizedGroupFileStorage)')
  } else {
    container.storage = await createStorageService()
    logger.info('Using default storage (FileStorage)')
  }

  container.github = getGitHubService()
  container.settings = await createSettingsService(container.github)
  container.upload = createUploadService()
  container.figma = getFigmaService()

  // 2. Services with single or dual dependencies
  container.auth = createAuthService(container.storage, container.github)
  container.repository = createRepositoryService(container.storage)
  container.claude = createClaudeService(container.settings)
  container.command = createCommandService(container.storage)
  container.docker = createDockerService(container.settings)
  container.netlify = createNetlifyService()

  // 3. Services with multiple dependencies
  container.event = createEventService(container.storage)

  container.workspace = createWorkspaceService(
    container.repository,
    container.storage,
    container.event,
    container.docker,
    container.claude,
    container.previewRegistry,
  )

  // Initialize hostname proxy service if hostname routing is enabled
  const targetResolver = new RegistryTargetResolver(
    container.previewRegistry,
    logger.child({ service: 'hostname-resolver' }),
  )

  // Create hostname proxy service with registry-backed resolver (singleton)
  await HostnameProxyService.getInstance(logger, targetResolver)

  // Set claude service on repository service for suggested repositories
  container.repository.setClaudeService(container.claude)

  // Set settings service on repository service for GitHub cloning
  container.repository.setSettingsService(container.settings)

  // 4. Message service needs claude, upload, and storage services (create early for agent dependency)
  // Note: We'll set task service later after it's created to avoid circular dependency
  container.message = createMessageService(container.claude, container.storage, container.upload)

  // Agent service needs workspace, repository, storage, message, and upload services
  container.agent = createAgentService(
    container.workspace,
    container.repository,
    container.storage,
    container.message,
    container.upload,
  )

  // Screenshot service needs storage, event, repository, and workspace
  container.screenshot = createScreenshotService(
    container.storage,
    container.event,
    container.repository,
    container.workspace,
  )

  // Task service needs many dependencies including agent and screenshot services
  container.task = createTaskService(
    container.storage,
    container.event,
    container.workspace,
    container.command,
    container.repository,
    container.agent,
    container.screenshot,
  )

  // Now set the task service on message service to complete the circular dependency
  container.message.setTaskService(container.task)

  // Tool service needs task, workspace, and repository services
  container.tool = createToolService(container.task, container.workspace, container.repository)

  // Set global tool service instance for agent access
  const { setToolService } = await import('./tool.service.js')
  setToolService(container.tool)

  // MCP service needs tool, task, and repository services
  container.mcp = createMcpService(container.tool, container.task, container.repository)

  // Return fully initialized container
  return container as ServiceContainer
}
