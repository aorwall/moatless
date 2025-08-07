import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import type { CommandService } from './command.service.js'
import type { DockerService } from './docker.service.js'
import type { EventService } from './event.service.js'
import type { GitHubService } from './github.service.js'
import type { MessageService } from './message.service.js'
import type { PreviewEnvironmentService } from './preview-environment.service.js'
import type { RepositoryService } from './repository.service.js'
import type { SettingsService } from './settings.service.js'
import type { StorageBackend } from './storage.js'
import type { TaskService } from './task.service.js'
import type { WorkspaceService } from './workspace.service.js'

/**
 * Service container interface containing all application services.
 */
export interface ServiceContainer {
  claude: ClaudeService
  command: CommandService
  docker: DockerService
  event: EventService
  github: GitHubService
  message: MessageService
  previewEnvironment: PreviewEnvironmentService
  repository: RepositoryService
  settings: SettingsService
  storage: StorageBackend
  task: TaskService
  workspace: WorkspaceService
}

/**
 * Factory function type for creating services.
 */
export type ServiceFactory<T> = (container: Partial<ServiceContainer>) => T

/**
 * Creates a fully initialized service container with all dependencies properly wired.
 * This is the main entry point for dependency injection in the application.
 */
export async function createServiceContainer(): Promise<ServiceContainer> {
  // Import all service factories
  const { createStorageService } = await import('./storage.js')
  const { getGitHubService } = await import('./github.service.js')
  const { createSettingsService } = await import('./settings.service.js')
  const { createRepositoryService } = await import('./repository.service.js')
  const { createClaudeService } = await import('../agents/claude-code/claude.service.js')
  const { createCommandService } = await import('./command.service.js')
  const { createDockerService } = await import('./docker.service.js')
  const { createEventService } = await import('./event.service.js')
  const { createMessageService } = await import('./message.service.js')
  const { createPreviewEnvironmentService } = await import('./preview-environment.service.js')
  const { createTaskService } = await import('./task.service.js')
  const { createWorkspaceService } = await import('./workspace.service.js')

  // Create container object that will be populated
  const container: Partial<ServiceContainer> = {}

  // Create services in dependency order
  // 1. Core services with no dependencies
  container.storage = await createStorageService()
  container.github = getGitHubService()
  container.settings = await createSettingsService(container.github)

  // 2. Services with single dependencies
  container.repository = createRepositoryService(container.storage)
  container.claude = createClaudeService(container.settings)
  container.command = createCommandService(container.storage)
  container.docker = createDockerService(container.settings)

  // 3. Services with multiple dependencies
  container.event = createEventService(container.storage)

  container.workspace = createWorkspaceService(
    container.repository,
    container.storage,
    container.event,
    container.docker,
    container.claude,
  )
  container.previewEnvironment = createPreviewEnvironmentService(
    container.storage,
    container.workspace,
  )

  // Set preview environment service on workspace service
  container.workspace.setPreviewEnvironmentService(container.previewEnvironment)

  // Set workspace service on repository service for verification
  container.repository.setWorkspaceService(container.workspace)

  // Set claude service on repository service for suggested repositories
  container.repository.setClaudeService(container.claude)

  // Set settings service on repository service for GitHub cloning
  container.repository.setSettingsService(container.settings)

  // Task service needs many dependencies
  container.task = createTaskService(
    container.storage,
    container.event,
    container.workspace,
    container.command,
    container.repository,
  )

  // 7. Message service needs claude and task services
  container.message = createMessageService(container.claude, container.task)

  // Return fully initialized container
  return container as ServiceContainer
}
