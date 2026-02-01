/**
 * Task Feature Metrics
 *
 * Tracks task lifecycle events including creation, status transitions,
 * turn completions, and duration.
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
 * Total task turns
 * Labels: status (completed, failed), agent_type
 */
export const taskTurnTotal = createCounter({
  name: 'moatless_task_turn_total',
  help: 'Total number of task turns',
  labelNames: ['status', 'agent_type'] as const,
})

/**
 * Task turn duration in seconds
 * Labels: agent_type
 * Buckets: 5s to 20min for typical agent task durations
 */
export const taskTurnDurationSeconds = createHistogram({
  name: 'moatless_task_turn_duration_seconds',
  help: 'Task turn execution duration in seconds',
  labelNames: ['agent_type'] as const,
  buckets: [5, 10, 30, 60, 120, 300, 600, 1200],
})
