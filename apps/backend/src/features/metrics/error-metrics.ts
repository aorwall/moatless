/**
 * Error Metrics
 *
 * Centralized error tracking for monitoring system health
 * and identifying failure patterns across services.
 */

import { createCounter } from './metrics-registry'

/**
 * Total number of errors tracked
 * Labels:
 * - error_type: Type of error
 *   - event_handler_error: Errors in event subscription handlers
 *   - message_persistence_error: Failed message saves
 *   - provisioner_error: Workspace provisioning failures
 *   - database_error: Database operation failures
 *   - delivery_error: Event delivery failures
 * - service: Service where error occurred
 *   - message_service, event_service, event_poller, notify_listener,
 *   - task_repository, event_repository, etc.
 * - severity: Error severity level
 *   - warning: Recoverable errors (logged as warn)
 *   - error: Non-recoverable errors (logged as error)
 *   - critical: System-impacting errors
 */
export const errorTotal = createCounter({
  name: 'moatless_error_total',
  help: 'Total number of errors',
  labelNames: ['error_type', 'service', 'severity'] as const,
})
