import type { FastifyInstance } from 'fastify'
import { type AgentService, createAgentService } from './agent.service.js'
import { type CommandService, createCommandService } from './command.service.js'
import { createEnvironmentService, type EnvironmentService } from './environment.service.js'
import { createEventService, type EventService } from './event.service.js'
import {
  createPreviewEnvironmentService,
  type PreviewEnvironmentService,
} from './preview-environment.service.js'
import { createStorageService, type StorageBackend } from './storage.js'
import { createTaskService, type TaskService } from './task.service.js'
import { createWorkspaceService, type WorkspaceService } from './workspace.service.js'

export interface Services {
  storage: StorageBackend
  event: EventService
  environment: EnvironmentService
  command: CommandService
  workspace: WorkspaceService
  previewEnvironment: PreviewEnvironmentService
  agent: AgentService
  task: TaskService
}

export async function setupServices(app: FastifyInstance): Promise<Services> {
  // Create services with dependency injection
  const storage = await createStorageService(app)
  const event = createEventService(app, storage)
  const environment = createEnvironmentService(app, storage)
  const command = createCommandService(app, storage)
  const workspace = createWorkspaceService(app, environment, event, storage)
  const previewEnvironment = createPreviewEnvironmentService(app, storage, workspace)

  // Set preview environment service on workspace service
  workspace.setPreviewEnvironmentService(previewEnvironment)

  const agent = createAgentService(app, storage, event, workspace)
  const task = createTaskService(app, storage, event, workspace, agent, command)

  return {
    storage,
    event,
    environment,
    command,
    workspace,
    previewEnvironment,
    agent,
    task,
  }
}

// Augment Fastify instance with services
declare module 'fastify' {
  interface FastifyInstance {
    services: Services
  }
}

// Export all services and types
export * from './agent.service.js'
export * from './agents/base-agent.js'
export { ClaudeCodeAgent } from './agents/claude-code-agent.js'
export * from './command.service.js'
export * from './environment.service.js'
export * from './event.service.js'
export * from './preview-environment.service.js'
export * from './storage.js'
export * from './task.service.js'
export * from './workspace.service.js'
