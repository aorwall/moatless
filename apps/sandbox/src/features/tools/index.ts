export type { SandboxTool, SandboxToolResult } from './types'
export { createUpdateTaskMetadataTool } from './update-task-metadata'

import type { SandboxConfig } from '../../config/environment'
import type { SandboxEventService } from '../events/services/sandbox-event.service'
import type { SandboxTool } from './types'
import { createUpdateTaskMetadataTool } from './update-task-metadata'

/**
 * Create all sandbox tools for the given config and event service.
 */
export function createSandboxTools(
  config: SandboxConfig,
  eventService: SandboxEventService | undefined,
): SandboxTool[] {
  return [createUpdateTaskMetadataTool(config, eventService)]
}
