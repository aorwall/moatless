/**
 * Tasks Feature Module
 *
 * This module owns task lifecycle, agent orchestration, messaging, and command execution.
 * It follows the feature-based architecture pattern with focused services and storage adapters.
 *
 * @see docs/patterns/backend-feature-architecture.md
 */

// Routes
export { registerTaskRoutes } from './api/tasks.routes'
export { registerWorkerRoutes } from './api/worker.routes'
// Services
export { MessageService } from './services/message.service'
export { TaskService } from './services/task.service'
export { TaskCommandGateway } from './services/task-command-gateway.service'
export { TaskEventProjector } from './services/task-event-projector.service'
export { TaskExecutionCoordinator } from './services/task-execution-coordinator.service'
export { TaskIterationService } from './services/task-iteration.service'
// Storage adapters
export { MessageStorage, TaskMetadataStorage } from './storage'
// Utilities
export { deriveTaskStatus } from './utils/task-status.utils'
