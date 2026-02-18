import type { SandboxEvent } from '../types'

/**
 * Get the source label to display for an event.
 * Server events show the server name (e.g., "preview"); sandbox events show "sandbox".
 */
export function getSourceLabel(event: SandboxEvent): string {
  if (event.type === 'server' && event.serverName) {
    return event.serverName
  }
  return event.type
}

/**
 * Generate a human-readable display message for an event.
 * Prefers backend-provided message, then reason, then generates a fallback
 * from the event type and subType.
 */
export function getDisplayMessage(event: SandboxEvent): string {
  if (event.message) {
    return event.message
  }
  if (event.reason) {
    return event.reason
  }
  // Fallback: humanize the subType
  return event.subType.replace(/_/g, ' ')
}
