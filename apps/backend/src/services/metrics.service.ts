/**
 * Metrics Service
 *
 * Central export for all Prometheus metrics.
 * Re-exports from metrics/ module for unified access.
 *
 * Metrics are opt-in via METRICS_ENABLED=true environment variable.
 * When disabled, no-op implementations are used with zero overhead.
 */

// Re-export everything from metrics module
export * from '../features/metrics'

import client from 'prom-client'
// Legacy exports for backward compatibility
// These use the new registry when METRICS_ENABLED=true
import {
  createCounter,
  createGauge,
  createHistogram,
  metricsEnabled,
  register,
} from '../features/metrics'

// Legacy metrics - kept for backward compatibility
// Note: When METRICS_ENABLED is false, these return no-op implementations
export const taskExecutionCounter = createCounter({
  name: 'moatless_task_executions_total',
  help: 'Total number of task executions',
  labelNames: ['status', 'agent_type'] as const,
})

export const taskDurationHistogram = createHistogram({
  name: 'moatless_task_duration_seconds',
  help: 'Task execution duration in seconds',
  labelNames: ['agent_type'] as const,
  buckets: [1, 5, 10, 30, 60, 120, 300, 600],
})

export const activeTasksGauge = createGauge({
  name: 'moatless_active_tasks',
  help: 'Number of currently active tasks',
})

export const activeWorkspacesGauge = createGauge({
  name: 'moatless_active_workspaces',
  help: 'Number of currently active workspaces (legacy)',
})

export const sseConnectionsGauge = createGauge({
  name: 'moatless_sse_connections',
  help: 'Number of active SSE connections',
})

// Re-export register for /metrics endpoint
// Uses the new registry from metrics module (or null when disabled)
export { register, metricsEnabled }

// Helper to get metrics output for /metrics endpoint
export async function getMetricsOutput(): Promise<string | null> {
  if (!(metricsEnabled && register)) {
    return null
  }
  return await register.metrics()
}

// Get content type for metrics endpoint
export function getMetricsContentType(): string {
  return client.register.contentType
}
