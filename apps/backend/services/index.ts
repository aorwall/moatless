import type { CommandService } from './command.service.js'
import type { EventService } from './event.service.js'
import type { PreviewEnvironmentService } from './preview-environment.service.js'
import type { RepositoryService } from './repository.service.js'
import type { SettingsService } from './settings.service.js'
import type { StorageBackend } from './storage.js'
import type { TaskService } from './task.service.js'
import type { WorkspaceService } from './workspace.service.js'

export interface Services {
  settings: SettingsService
  storage: StorageBackend
  event: EventService
  repository: RepositoryService
  command: CommandService
  workspace: WorkspaceService
  previewEnvironment: PreviewEnvironmentService
  task: TaskService
}

// This function is deprecated - use createServiceContainer from service-container.ts instead
export async function setupServices(): Promise<Services> {
  throw new Error(
    'setupServices is deprecated. Use createServiceContainer from service-container.ts instead',
  )
}

export * from '../agents/base-agent.js'
export * from '../agents/claude-code/claude.service.js'
export { ClaudeCodeAgent } from '../agents/claude-code/claude-code-agent.js'
// Export all services and types
export * from './command.service.js'
export * from './event.service.js'
export * from './preview-environment.service.js'
export * from './repository.service.js'
export * from './settings.service.js'
export * from './storage.js'
export * from './task.service.js'
export * from './workspace.service.js'
