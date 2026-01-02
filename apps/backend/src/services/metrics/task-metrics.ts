/**
 * Task Feature Metrics
 *
 * Tracks task lifecycle events including creation, status transitions,
 * iteration completions, and duration.
 */

import { createCounter, createHistogram } from './metrics-registry'

/**
 * Total tasks created
 * Labels: agent_type
 */
export const taskCreatedTotal = createCounter({
  name: 'moatless_task_created_total',
  help: 'Total number of tasks created',
  labelNames: ['agent_type'] as const,
})

/**
 * Task status transitions
 * Labels: status (open, running, completed, error, closed), agent_type
 */
export const taskStatusTransitionsTotal = createCounter({
  name: 'moatless_task_status_transitions_total',
  help: 'Total number of task status transitions',
  labelNames: ['status', 'agent_type'] as const,
})

/**
 * Total task iterations
 * Labels: status (created, completed, failed), agent_type
 */
export const taskIterationTotal = createCounter({
  name: 'moatless_task_iteration_total',
  help: 'Total number of task iterations',
  labelNames: ['status', 'agent_type'] as const,
})

/**
 * Task iteration duration in seconds
 * Labels: agent_type
 * Buckets: 5s to 20min for typical agent task durations
 */
export const taskIterationDurationSeconds = createHistogram({
  name: 'moatless_task_iteration_duration_seconds',
  help: 'Task iteration execution duration in seconds',
  labelNames: ['agent_type'] as const,
  buckets: [5, 10, 30, 60, 120, 300, 600, 1200],
})
