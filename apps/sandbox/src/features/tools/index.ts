export type { SandboxTool, SandboxToolResult } from './types'
export { createUpdateTaskMetadataTool } from './update-task-metadata'

import type { SandboxConfig } from '../../config/environment'
import type { BackendApiClient } from '../../shared/backend-api-client'
import type { SandboxTool } from './types'
import { createUpdateTaskMetadataTool } from './update-task-metadata'

/**
 * Create all sandbox tools for the given config and backend API client.
 */
export function createSandboxTools(
  config: SandboxConfig,
  backendApi: BackendApiClient | undefined,
): SandboxTool[] {
  return [createUpdateTaskMetadataTool(config, backendApi)]
}
