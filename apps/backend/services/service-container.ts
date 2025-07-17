import type { RepositoryConfig } from '@moatless/types'
import type { AgentService } from './agent.service.js'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import type { CommandService } from './command.service.js'
import type { EventService } from './event.service.js'
import type { FileOperations } from './file-operations.service.js'
import type { MessageService } from './message.service.js'
import type { PreviewEnvironmentService } from './preview-environment.service.js'
import type { RepositoryService } from './repository.service.js'
import type { SettingsService } from './settings.service.js'
import type { StorageBackend } from './storage.js'
import type { TaskService } from './task.service.js'
import type { WorkspaceService } from './workspace.service.js'

/**
 * Service container interface containing all application services.
 * This ensures type safety and makes dependencies explicit.
 */
export interface ServiceContainer {
  agent: AgentService
  claude: ClaudeService
  command: CommandService
  event: EventService
  fileOperations: FileOperationsService
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
 * Used to ensure consistent service creation patterns.
 */
export type ServiceFactory<T> = (container: Partial<ServiceContainer>) => T

/**
 * Configuration for creating a service container.
 * Includes all necessary parameters for service initialization.
 */
export interface ServiceContainerConfig {
  dataDir: string
  settingsPath?: string
  logsDir?: string
}

/**
 * Creates a fully initialized service container with all dependencies properly wired.
 * This is the main entry point for dependency injection in the application.
 */
export async function createServiceContainer(config: ServiceContainerConfig): Promise<ServiceContainer> {
  // Import all service factories
  const { createStorageService } = await import('./storage.js')
  const { createSettingsService } = await import('./settings.service.js')
  const { createRepositoryService } = await import('./repository.service.js')
  const { createAgentService } = await import('./agent.service.js')
  const { createClaudeService } = await import('../agents/claude-code/claude.service.js')
  const { createCommandService } = await import('./command.service.js')
  const { createEventService } = await import('./event.service.js')
  const { createFileOperationsService } = await import('./file-operations.service.js')
  const { createMessageService } = await import('./message.service.js')
  const { createPreviewEnvironmentService } = await import('./preview-environment.service.js')
  const { createTaskService } = await import('./task.service.js')
  const { createWorkspaceService } = await import('./workspace.service.js')

  // Create container object that will be populated
  const container: Partial<ServiceContainer> = {}

  // Create services in dependency order
  // 1. Core services with no dependencies
  container.storage = await createStorageService()
  container.settings = await createSettingsService(config.settingsPath)

  // 2. Services with single dependencies
  container.repository = createRepositoryService(container.storage)
  container.claude = createClaudeService(container.settings)
  container.command = createCommandService(container.storage)
  container.fileOperations = createFileOperationsService()

  // 3. Services with multiple dependencies
  container.workspace = createWorkspaceService(container.repository, container.storage)
  container.previewEnvironment = createPreviewEnvironmentService(
    container.storage,
    container.workspace
  )
  
  // Set preview environment service on workspace service
  container.workspace.setPreviewEnvironmentService(container.previewEnvironment)
  
  // Set workspace service on repository service for verification
  container.repository.setWorkspaceService(container.workspace)
  
  // 4. Event service needs claude service
  container.event = createEventService(container.storage, container.claude)

  // 5. Services that need event and workspace
  container.agent = createAgentService(container.storage, container.event, container.workspace)
  
  // 6. Task service needs many dependencies
  container.task = createTaskService(
    container.storage,
    container.event,
    container.workspace,
    container.agent,
    container.command,
    container.repository
  )

  // 7. Message service needs storage, claude, and repository
  container.message = createMessageService(
    container.claude,
    container.repository
  )

  // Return fully initialized container
  return container as ServiceContainer
}

/**
 * Helper function to get a service from the container with type safety.
 * Throws if the service is not initialized.
 */
export function getService<K extends keyof ServiceContainer>(
  container: ServiceContainer,
  serviceName: K
): ServiceContainer[K] {
  const service = container[serviceName]
  if (!service) {
    throw new Error(`Service '${serviceName}' not initialized in container`)
  }
  return service
}