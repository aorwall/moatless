/**
 * Runtime Health Metrics
 *
 * Live-verified health metrics for workspaces, preview servers, and agents.
 * These metrics are collected periodically by RuntimeHealthCollector and reflect
 * actual resource state (not just DB state).
 */

import { createCounter, createGauge, createHistogram } from './metrics-registry'

/**
 * Live-verified workspace/pod health by mode and state
 * Labels:
 *   - mode: 'local' | 'kubernetes'
 *   - state: 'available' | 'unavailable' (local) or pod phase (kubernetes)
 */
export const workspaceHealth = createGauge({
  name: 'moatless_workspace_health',
  help: 'Number of workspaces by mode and verified state',
  labelNames: ['mode', 'state'] as const,
})

/**
 * Live-verified preview server health by status
 * Labels:
 *   - status: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
 */
export const previewHealth = createGauge({
  name: 'moatless_preview_health',
  help: 'Number of preview servers by verified status',
  labelNames: ['status'] as const,
})

/**
 * Live-verified agent health by state
 * Labels:
 *   - state: 'idle' | 'running'
 */
export const agentHealth = createGauge({
  name: 'moatless_agent_health',
  help: 'Number of agents by verified state',
  labelNames: ['state'] as const,
})

/**
 * Health collector duration by type
 * Tracks how long each collection cycle takes
 * Labels:
 *   - type: 'workspace' | 'preview' | 'agent'
 */
export const healthCollectorDuration = createHistogram({
  name: 'moatless_health_collector_duration_seconds',
  help: 'Duration of health collection by type',
  labelNames: ['type'] as const,
  buckets: [0.1, 0.5, 1, 2, 5, 10],
})

/**
 * Health collector errors by type
 * Tracks failures during health collection
 * Labels:
 *   - type: 'workspace' | 'preview' | 'agent'
 */
export const healthCollectorErrors = createCounter({
  name: 'moatless_health_collector_errors_total',
  help: 'Errors during health collection',
  labelNames: ['type'] as const,
})
