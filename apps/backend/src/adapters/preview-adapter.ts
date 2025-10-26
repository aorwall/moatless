import type { PreviewServerConfig, PreviewServerStatus } from '@moatless/preview-server'
import type { RepositoryConfig, ServerStatus } from '@moatless/types'

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
    setupCommands: repoConfig.codeExecution.setupCommands,
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
