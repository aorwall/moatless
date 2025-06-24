import type { FastifyInstance } from 'fastify'
import { type AgentService, createAgentService } from './agent.js'
import { createEnvironmentService, type EnvironmentService } from './environment.js'
import { createEventService, type EventService } from './event.js'
import { createStorageService, type StorageBackend } from './storage.js'
import { createTaskService, type TaskService } from './task.js'
import { createWorkspaceService, type WorkspaceService } from './workspace.js'

export interface Services {
  storage: StorageBackend
  event: EventService
  environment: EnvironmentService
  workspace: WorkspaceService
  task: TaskService
  agent: AgentService
}

export function setupServices(app: FastifyInstance): Services {
  // Create services with dependency injection
  const storage = createStorageService(app)
  const event = createEventService(app, storage)
  const environment = createEnvironmentService(app, storage)
  const workspace = createWorkspaceService(app, storage, environment, event)
  const task = createTaskService(app, storage, event, environment, workspace)
  const agent = createAgentService(app, task, workspace, event)

  return {
    storage,
    event,
    environment,
    workspace,
    task,
    agent,
  }
}

// Augment Fastify instance with services
declare module 'fastify' {
  interface FastifyInstance {
    services: Services
  }
}

export * from './agent.js'
export * from './agents/agent-options.js'
export * from './agents/base-agent.js'
export { ClaudeCodeAgent } from './agents/claude-code-agent.js'
export * from './environment.js'
export * from './event.js'
// Export all services and types
export * from './storage.js'
export * from './task.js'
export * from './workspace.js'
