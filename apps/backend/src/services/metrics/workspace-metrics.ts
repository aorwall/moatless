/**
 * Workspace Feature Metrics
 *
 * Tracks workspace provisioning events including creation, mode,
 * and Kubernetes pod lifecycle.
 */

import { createCounter, createGauge, createHistogram } from './metrics-registry'

/**
 * Total workspace provisioning attempts
 * Labels: mode (direct, worktree, clone, kubernetes), status (success, error)
 */
export const workspaceProvisionTotal = createCounter({
  name: 'moatless_workspace_provision_total',
  help: 'Total number of workspace provisioning attempts',
  labelNames: ['mode', 'status'] as const,
})

/**
 * Workspace provisioning duration in seconds
 * Labels: mode
 * Buckets: 1s to 3min for typical provisioning times
 */
export const workspaceProvisionDurationSeconds = createHistogram({
  name: 'moatless_workspace_provision_duration_seconds',
  help: 'Time to provision workspace in seconds',
  labelNames: ['mode'] as const,
  buckets: [1, 5, 10, 30, 60, 120, 180],
})

/**
 * Currently active workspaces
 * Labels: mode
 */
export const activeWorkspaces = createGauge({
  name: 'moatless_active_workspaces',
  help: 'Number of currently active workspaces',
  labelNames: ['mode'] as const,
})

/**
 * Kubernetes pod lifecycle events
 * Labels: status (created, running, ready, failed, deleted)
 */
export const kubernetesPodTotal = createCounter({
  name: 'moatless_kubernetes_pod_total',
  help: 'Total number of Kubernetes pod lifecycle events',
  labelNames: ['status'] as const,
})

/**
 * Kubernetes pod readiness duration in seconds
 * Buckets: 5s to 5min for pod startup times
 */
export const kubernetesPodReadinessDurationSeconds = createHistogram({
  name: 'moatless_kubernetes_pod_readiness_duration_seconds',
  help: 'Time for Kubernetes pod to become ready in seconds',
  buckets: [5, 10, 30, 60, 120, 180, 300],
})

// =============================================================================
// Provisioner Metrics
// =============================================================================

/**
 * Total provisioner health checks
 * Labels: mode (local, kubernetes), result (healthy, unhealthy, error)
 */
export const provisionerHealthCheckTotal = createCounter({
  name: 'moatless_provisioner_health_check_total',
  help: 'Total provisioner health checks',
  labelNames: ['mode', 'result'] as const,
})

/**
 * Provisioner health check duration in seconds
 * Labels: mode
 * Buckets: 50ms to 5s for health check response times
 */
export const provisionerHealthCheckDurationSeconds = createHistogram({
  name: 'moatless_provisioner_health_check_duration_seconds',
  help: 'Duration of provisioner health checks in seconds',
  labelNames: ['mode'] as const,
  buckets: [0.05, 0.1, 0.25, 0.5, 1, 2, 5],
})

/**
 * Total provisioner validations
 * Labels: mode (local, kubernetes), result (valid, invalid, error)
 */
export const provisionerValidateTotal = createCounter({
  name: 'moatless_provisioner_validate_total',
  help: 'Total provisioner validations',
  labelNames: ['mode', 'result'] as const,
})

/**
 * Total provisioner cleanup operations
 * Labels: mode (local, kubernetes), status (success, error)
 */
export const provisionerCleanupTotal = createCounter({
  name: 'moatless_provisioner_cleanup_total',
  help: 'Total provisioner cleanup operations',
  labelNames: ['mode', 'status'] as const,
})

/**
 * Total drivers created via provisioner
 * Labels: mode (local, kubernetes)
 */
export const provisionerDriverCreatedTotal = createCounter({
  name: 'moatless_provisioner_driver_created_total',
  help: 'Total drivers created via provisioner',
  labelNames: ['mode'] as const,
})

/**
 * Driver cache hit/miss tracking
 * Labels: result (hit_healthy, hit_stale, miss)
 */
export const driverCacheHitTotal = createCounter({
  name: 'moatless_driver_cache_hit_total',
  help: 'Driver cache hit/miss events',
  labelNames: ['result'] as const,
})
