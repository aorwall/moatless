/**
 * Metrics Module
 *
 * Central export for all Prometheus metrics.
 * Metrics are opt-in via METRICS_ENABLED=true environment variable.
 */

// Agent metrics
export {
  activeAgents,
  agentExecutionDurationSeconds,
  agentExecutionTotal,
  agentMessageTotal,
  toolInvocationTotal,
} from './agent-metrics'
// HTTP metrics
export { httpRequestDurationSeconds, httpRequestTotal, normalizePath } from './http-metrics'
// Registry and factory functions
export {
  createCounter,
  createGauge,
  createHistogram,
  metricsEnabled,
  register,
} from './metrics-registry'

// Preview metrics
export {
  activePreviewServers,
  previewServerHttpWaitDurationSeconds,
  previewServerStartDurationSeconds,
  previewServerTotal,
} from './preview-metrics'
// Task metrics
export {
  taskCreatedTotal,
  taskIterationDurationSeconds,
  taskIterationTotal,
  taskStatusTransitionsTotal,
} from './task-metrics'
// Workspace metrics
export {
  activeWorkspaces,
  kubernetesPodReadinessDurationSeconds,
  kubernetesPodTotal,
  workspaceProvisionDurationSeconds,
  workspaceProvisionTotal,
} from './workspace-metrics'
