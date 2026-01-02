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
