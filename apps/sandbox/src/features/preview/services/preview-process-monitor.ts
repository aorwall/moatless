/**
 * PreviewProcessMonitor - Backward-compatible wrapper around ServerProcessMonitor
 *
 * Delegates to the generic ServerProcessMonitor with name='preview'.
 * Kept for backward compatibility with existing code that imports from this module.
 */

import type { SandboxConfig } from '../../../config/environment'
import type { SandboxEventService } from '../../events/services/sandbox-event.service'
import { ServerProcessMonitor } from '../../servers/services/server-process-monitor'

export class PreviewProcessMonitor extends ServerProcessMonitor {
  constructor(config: SandboxConfig, eventService?: SandboxEventService) {
    super('preview', config, eventService)
  }
}

// Module-scoped monitor instance (backward compat)
let monitorInstance: PreviewProcessMonitor | undefined

/**
 * Initialize the preview process monitor
 */
export function initializeMonitor(
  config: SandboxConfig,
  eventService?: SandboxEventService,
): PreviewProcessMonitor {
  monitorInstance = new PreviewProcessMonitor(config, eventService)
  return monitorInstance
}

/**
 * Get the monitor instance (or undefined if not initialized)
 */
export function getMonitorOrUndefined(): PreviewProcessMonitor | undefined {
  return monitorInstance
}
