import type { ServiceIdentifier } from 'inversify'
import type { TaskExecutionCoordinator } from './services/task-execution-coordinator.service'

/**
 * DI symbol for TaskExecutionCoordinator.
 * Used to break circular module loading issues when injecting TaskExecutionCoordinator
 * into TaskEventHandler.
 */
export const TaskExecutionCoordinatorSymbol: ServiceIdentifier<TaskExecutionCoordinator> =
  Symbol.for('TaskExecutionCoordinator')
