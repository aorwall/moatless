import {
  getPreviewInspectorScript,
  type PreviewServerConfig,
  type PreviewServerStatus,
  type TargetInfo,
  type TargetResolver,
} from '@moatless/preview-server'
import type { RepositoryConfig, ServerStatus } from '@moatless/types'
import type { WorkspaceService } from '../services/workspace.service.js'

/**
 * Convert RepositoryConfig → PreviewServerConfig
 * Maps moatless-specific configuration to generic preview server configuration
 */
export function toPreviewServerConfig(
  taskId: string,
  workspacePath: string,
  repoConfig: RepositoryConfig,
  logFile?: string,
): PreviewServerConfig {
  return {
    serverId: taskId, // Map moatless taskId to generic serverId
    workspacePath,
    serverCommands: repoConfig.previewConfig?.serverStartCommands || [],
    envVars: repoConfig.codeExecution.envVars,
    startupTimeoutSeconds: repoConfig.previewConfig?.startupTimeoutSeconds,
    enableInspector: repoConfig.previewConfig?.enableInspector,
    logFile,
  }
}

/**
 * Convert PreviewServerStatus → ServerStatus
 * Maps generic preview server status to moatless-specific status
 */
export function toServerStatus(status: PreviewServerStatus): ServerStatus {
  return {
    status: status.status,
    pid: status.pid,
    url: status.proxyUrl || status.url, // Use proxy URL if available, otherwise direct server URL
    uptimeSeconds: status.uptimeSeconds || 0,
    currentPath: status.currentPath,
    error: status.error,
    logs: status.logs,
  }
}

/**
 * Resolver implementation for hostname proxy
 * Implements TargetResolver interface using WorkspaceService
 */
export class WorkspaceTargetResolver implements TargetResolver {
  constructor(private workspaceService: WorkspaceService) {}

  async resolve(taskId: string): Promise<TargetInfo | null> {
    const workspace = await this.workspaceService.getWorkspace(taskId)
    if (!workspace) {
      return null
    }

    const status = await workspace.getServerStatus()
    if (status.status !== 'started' || !status.url) {
      return null
    }

    const port = Number.parseInt(new URL(status.url).port, 10)
    if (!port || Number.isNaN(port)) {
      return null
    }

    const enableInspector = workspace.environmentConfig.previewConfig?.enableInspector !== false

    return {
      port,
      scriptContent: enableInspector ? getPreviewInspectorScript(taskId) : undefined,
    }
  }
}
