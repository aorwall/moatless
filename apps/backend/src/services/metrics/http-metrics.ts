/**
 * HTTP Request Metrics
 *
 * Tracks HTTP request counts and latencies by method, path, and status code.
 */

import { createCounter, createHistogram } from './metrics-registry'

/**
 * Total HTTP requests
 * Labels: method, path, status_code
 */
export const httpRequestTotal = createCounter({
  name: 'moatless_http_request_total',
  help: 'Total number of HTTP requests',
  labelNames: ['method', 'path', 'status_code'] as const,
})

/**
 * HTTP request duration in seconds
 * Labels: method, path
 * Buckets: 10ms to 5s for typical API response times
 */
export const httpRequestDurationSeconds = createHistogram({
  name: 'moatless_http_request_duration_seconds',
  help: 'HTTP request duration in seconds',
  labelNames: ['method', 'path'] as const,
  buckets: [0.01, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5],
})

/**
 * Normalize URL path to reduce cardinality.
 * Replaces UUIDs and numeric IDs with placeholder.
 */
export function normalizePath(path: string): string {
  return (
    path
      // Replace UUIDs (e.g., 550e8400-e29b-41d4-a716-446655440000)
      .replace(/\/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, '/:id')
      // Replace numeric IDs
      .replace(/\/\d+/g, '/:id')
      // Remove trailing slashes
      .replace(/\/+$/, '')
  )
}
