import type { FastifyInstance } from 'fastify'
import { type AgentService, createAgentService } from './agent.service.js'
import { createEnvironmentService, type EnvironmentService } from './environment.service.js'
import { createEventService, type EventService } from './event.service.js'
import { createStorageService, type StorageBackend } from './storage.js'
import { createTaskService, type TaskService } from './task.service.js'
import { createWorkspaceService, type WorkspaceService } from './workspace.service.js'

export interface Services {
  storage: StorageBackend
  event: EventService
  environment: EnvironmentService
  workspace: WorkspaceService
  agent: AgentService
  task: TaskService
}

export async function setupServices(app: FastifyInstance): Promise<Services> {
  // Create services with dependency injection
  const storage = await createStorageService(app)
  const event = createEventService(app, storage)
  const environment = createEnvironmentService(app, storage)
  const workspace = createWorkspaceService(app, environment, event, storage)
  const agent = createAgentService(app, storage, event, workspace)
  const task = createTaskService(app, storage, event, workspace, agent)

  return {
    storage,
    event,
    environment,
    workspace,
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
export * from './environment.service.js'
export * from './event.service.js'
export * from './storage.js'
export * from './task.service.js'
export * from './workspace.service.js'
