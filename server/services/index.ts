import type { FastifyInstance } from 'fastify'
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
}

export function setupServices(app: FastifyInstance): Services {
  // Create services with dependency injection
  const storage = createStorageService(app)
  const event = createEventService(app, storage)
  const environment = createEnvironmentService(app, storage)
  const workspace = createWorkspaceService(app, storage, environment, event)
  const task = createTaskService(app, storage, event, environment, workspace)

  return {
    storage,
    event,
    environment,
    workspace,
    task,
  }
}

// Augment Fastify instance with services
declare module 'fastify' {
  interface FastifyInstance {
    services: Services
  }
}
