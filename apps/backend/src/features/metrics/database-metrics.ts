/**
 * Database Metrics
 *
 * Tracks database operation performance for monitoring
 * query latency and identifying slow operations.
 */

import { createCounter, createHistogram } from './metrics-registry'

/**
 * Database operation duration in seconds
 * Labels:
 * - operation: Type of operation (select, insert, update, delete, upsert)
 * - table: Database table name (task_events, tasks, task_iterations, etc.)
 */
export const databaseOperationDurationSeconds = createHistogram({
  name: 'moatless_database_operation_duration_seconds',
  help: 'Database operation duration in seconds',
  labelNames: ['operation', 'table'] as const,
  buckets: [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5],
})

/**
 * Total number of database operations
 * Labels:
 * - operation: Type of operation (select, insert, update, delete, upsert)
 * - table: Database table name
 * - status: Operation result (success, error)
 */
export const databaseOperationTotal = createCounter({
  name: 'moatless_database_operation_total',
  help: 'Total number of database operations',
  labelNames: ['operation', 'table', 'status'] as const,
})
