/**
 * HTTP Metrics Middleware
 *
 * Tracks HTTP request counts and latencies for Prometheus monitoring.
 * Only active when METRICS_ENABLED=true.
 */

import type { MiddlewareHandler } from 'hono'
import { httpRequestDurationSeconds, httpRequestTotal, normalizePath } from '../services/metrics'

/**
 * HTTP metrics middleware that tracks:
 * - Request count by method, path, and status code
 * - Request duration by method and path
 *
 * Paths are normalized to reduce cardinality (UUIDs replaced with :id)
 */
export const httpMetrics: MiddlewareHandler = async (c, next) => {
  const startTime = Date.now()
  const method = c.req.method
  const path = normalizePath(c.req.path)

  await next()

  const statusCode = c.res.status.toString()
  const durationSeconds = (Date.now() - startTime) / 1000

  httpRequestTotal.labels({ method, path, status_code: statusCode }).inc()
  httpRequestDurationSeconds.labels({ method, path }).observe(durationSeconds)
}
