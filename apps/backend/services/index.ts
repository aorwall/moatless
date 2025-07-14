import { type AgentService, createAgentService } from './agent.service.js'
import { type CommandService, createCommandService } from './command.service.js'
import { createEventService, type EventService } from './event.service.js'
import {
  createPreviewEnvironmentService,
  type PreviewEnvironmentService,
} from './preview-environment.service.js'
import { createRepositoryService, type RepositoryService } from './repository.service.js'
import { initializeSettingsService, type SettingsService } from './settings.service.js'
import { createStorageService, type StorageBackend } from './storage.js'
import { createTaskService, type TaskService } from './task.service.js'
import { createWorkspaceService, type WorkspaceService } from './workspace.service.js'

export interface Services {
  settings: SettingsService
  storage: StorageBackend
  event: EventService
  repository: RepositoryService
  command: CommandService
  workspace: WorkspaceService
  previewEnvironment: PreviewEnvironmentService
  agent: AgentService
  task: TaskService
}

export async function setupServices(): Promise<Services> {
  // Initialize settings service first
  const settings = await initializeSettingsService()

  // Create services with dependency injection
  const storage = await createStorageService()
  const event = createEventService(storage)
  const repository = createRepositoryService(storage)
  const command = createCommandService(storage)
  const workspace = createWorkspaceService(repository, event, storage)
  const previewEnvironment = createPreviewEnvironmentService(storage, workspace)

  // Set preview environment service on workspace service
  workspace.setPreviewEnvironmentService(previewEnvironment)

  const agent = createAgentService(storage, event, workspace)
  const task = createTaskService(storage, event, workspace, agent, command, repository)

  return {
    settings,
    storage,
    event,
    repository,
    command,
    workspace,
    previewEnvironment,
    agent,
    task,
  }
}

export * from '../agents/base-agent.js'
export { ClaudeCodeAgent } from '../agents/claude-code/claude-code-agent.js'
// Export all services and types
export * from './agent.service.js'
export * from './command.service.js'
export * from './event.service.js'
export * from './preview-environment.service.js'
export * from './repository.service.js'
export * from './settings.service.js'
export * from './storage.js'
export * from './task.service.js'
export * from './workspace.service.js'
