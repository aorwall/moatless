/**
 * Metrics Registry
 *
 * Central registry for all Prometheus metrics with opt-in behavior.
 * Metrics are only collected when METRICS_ENABLED=true environment variable is set.
 */

import client from 'prom-client'
import {
  createNoOpCounter,
  createNoOpGauge,
  createNoOpHistogram,
  type NoOpCounter,
  type NoOpGauge,
  type NoOpHistogram,
} from './no-op-metrics'

export const metricsEnabled = process.env['METRICS_ENABLED'] === 'true'

// Create registry only when metrics are enabled
const registry = metricsEnabled ? new client.Registry() : null

// Add default metrics when enabled
if (metricsEnabled && registry) {
  client.collectDefaultMetrics({
    register: registry,
    prefix: 'moatless_',
  })
}

export { registry as register }

/**
 * Create a Counter metric.
 * Returns a no-op counter when metrics are disabled.
 */
export function createCounter<T extends string>(
  options: client.CounterConfiguration<T>,
): client.Counter<T> | NoOpCounter {
  if (!(metricsEnabled && registry)) {
    return createNoOpCounter()
  }
  return new client.Counter({
    ...options,
    registers: [registry],
  })
}

/**
 * Create a Histogram metric.
 * Returns a no-op histogram when metrics are disabled.
 */
export function createHistogram<T extends string>(
  options: client.HistogramConfiguration<T>,
): client.Histogram<T> | NoOpHistogram {
  if (!(metricsEnabled && registry)) {
    return createNoOpHistogram()
  }
  return new client.Histogram({
    ...options,
    registers: [registry],
  })
}

/**
 * Create a Gauge metric.
 * Returns a no-op gauge when metrics are disabled.
 */
export function createGauge<T extends string>(
  options: client.GaugeConfiguration<T>,
): client.Gauge<T> | NoOpGauge {
  if (!(metricsEnabled && registry)) {
    return createNoOpGauge()
  }
  return new client.Gauge({
    ...options,
    registers: [registry],
  })
}
