export { createDownloadFileTool } from './download-file'
export type { SandboxTool, SandboxToolResult } from './types'
export { createUpdateTaskMetadataTool } from './update-task-metadata'

import type { SandboxConfig } from '../../config/environment'
import type { BackendApiClient } from '../../shared/backend-api-client'
import { createDownloadFileTool } from './download-file'
import type { SandboxTool } from './types'
import { createUpdateTaskMetadataTool } from './update-task-metadata'

/**
 * Create all sandbox tools for the given config and backend API client.
 */
export function createSandboxTools(
  config: SandboxConfig,
  backendApi: BackendApiClient | undefined,
): SandboxTool[] {
  return [
    createUpdateTaskMetadataTool(config, backendApi),
    createDownloadFileTool(config, backendApi),
  ]
}
