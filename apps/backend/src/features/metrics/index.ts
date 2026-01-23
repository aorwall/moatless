/**
 * Metrics Module
 *
 * Central export for all Prometheus metrics.
 * Metrics are opt-in via METRICS_ENABLED=true environment variable.
 */

// Agent metrics
export {
  agentCostUsdTotal,
  agentExecutionDurationSeconds,
  agentExecutionTotal,
  agentInputTokensTotal,
  agentMessageTotal,
  agentOutputTokensTotal,
  toolInvocationTotal,
} from './agent-metrics'
// Database metrics
export { databaseOperationDurationSeconds, databaseOperationTotal } from './database-metrics'
// Error metrics
export { errorTotal } from './error-metrics'
// Event metrics
export {
  eventDeliveryLatencySeconds,
  eventDeliveryTotal,
  eventEmittedTotal,
  eventPollBatchSize,
  eventUndeliveredCount,
} from './event-metrics'
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
// Runtime health metrics (live-verified)
export {
  agentHealth,
  healthCollectorDuration,
  healthCollectorErrors,
  previewHealth,
  taskHealth,
  workspaceHealth,
} from './runtime-health-metrics'
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
  driverCacheHitTotal,
  kubernetesPodReadinessDurationSeconds,
  kubernetesPodTotal,
  kubernetesPvcCreationDurationSeconds,
  kubernetesPvcTotal,
  provisionerCleanupTotal,
  provisionerDriverCreatedTotal,
  provisionerGetStatusTotal,
  provisionerHealthCheckDurationSeconds,
  provisionerHealthCheckTotal,
  workspaceProvisionDurationSeconds,
  workspaceProvisionTotal,
} from './workspace-metrics'
