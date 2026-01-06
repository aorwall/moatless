/**
 * Preview Server Metrics
 *
 * Tracks preview server lifecycle including startup, shutdown,
 * HTTP readiness, and active server count.
 */

import { createCounter, createGauge, createHistogram } from './metrics-registry'

/**
 * Total preview server lifecycle events
 * Labels: status (starting, started, stopped, failed, restarted)
 */
export const previewServerTotal = createCounter({
  name: 'moatless_preview_server_total',
  help: 'Total number of preview server lifecycle events',
  labelNames: ['status'] as const,
})

/**
 * Preview server startup duration in seconds
 * Includes setup commands execution time
 * Buckets: 1s to 1min for typical server startup times
 */
export const previewServerStartDurationSeconds = createHistogram({
  name: 'moatless_preview_server_start_duration_seconds',
  help: 'Time to start preview server in seconds',
  buckets: [1, 3, 5, 10, 30, 60],
})

/**
 * Time waiting for HTTP readiness in seconds
 * Buckets: 0.5s to 30s for typical HTTP readiness checks
 */
export const previewServerHttpWaitDurationSeconds = createHistogram({
  name: 'moatless_preview_server_http_wait_duration_seconds',
  help: 'Time waiting for preview server HTTP readiness in seconds',
  buckets: [0.5, 1, 2, 5, 10, 30],
})

/**
 * Currently active preview servers
 */
export const activePreviewServers = createGauge({
  name: 'moatless_active_preview_servers',
  help: 'Number of currently running preview servers',
})
