/**
 * Tasks Feature Module
 *
 * This module owns task lifecycle, agent orchestration, and command execution.
 * It follows the feature-based architecture pattern with focused services and storage adapters.
 *
 * @see docs/patterns/backend-feature-architecture.md
 */

// Routes
export { registerTaskRoutes } from './api/tasks.routes'
export { TaskRepository } from './db/task.repository'
// Repositories (Drizzle-based)
export { TaskMessageRepository } from './db/task-message.repository'
// Services
export { TaskService } from './services/task.service'
export { TaskCommandGateway } from './services/task-command-gateway.service'
export { TaskExecutionCoordinator } from './services/task-execution-coordinator.service'
export { TaskIterationService } from './services/task-iteration.service'
// Utilities
export { deriveTaskStatus } from './utils/task-status.utils'
