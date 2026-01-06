/**
 * Event Processing Metrics
 *
 * Tracks event emission, delivery, and processing for monitoring
 * NOTIFY/LISTEN and polling-based event delivery.
 */

import { createCounter, createGauge, createHistogram } from './metrics-registry'

/**
 * Total number of events emitted
 * Labels:
 * - type: Event type (agent, task, server, preview, etc.)
 * - storage_mode: Storage backend (postgres, file)
 */
export const eventEmittedTotal = createCounter({
  name: 'moatless_event_emitted_total',
  help: 'Total number of events emitted',
  labelNames: ['type', 'storage_mode'] as const,
})

/**
 * Total number of event deliveries
 * Labels:
 * - mode: Delivery mode (notify, poll)
 * - status: Delivery status (success, error)
 */
export const eventDeliveryTotal = createCounter({
  name: 'moatless_event_delivery_total',
  help: 'Total number of event deliveries',
  labelNames: ['mode', 'status'] as const,
})

/**
 * Event delivery latency in seconds
 * Measures time from event notification/poll to successful delivery
 * Labels:
 * - mode: Delivery mode (notify, poll)
 */
export const eventDeliveryLatencySeconds = createHistogram({
  name: 'moatless_event_delivery_latency_seconds',
  help: 'Event delivery latency in seconds',
  labelNames: ['mode'] as const,
  buckets: [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5],
})

/**
 * Number of events processed per poll cycle
 * Helps monitor polling efficiency and batch sizes
 */
export const eventPollBatchSize = createHistogram({
  name: 'moatless_event_poll_batch_size',
  help: 'Number of events processed per poll cycle',
  buckets: [0, 1, 5, 10, 25, 50, 100],
})

/**
 * Current count of undelivered events in database
 * Useful for monitoring event backlog
 */
export const eventUndeliveredCount = createGauge({
  name: 'moatless_event_undelivered_count',
  help: 'Current count of undelivered events in database',
})
