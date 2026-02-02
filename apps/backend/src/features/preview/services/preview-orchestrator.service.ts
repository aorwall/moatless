import type { ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { WorkspaceHandle } from '../../workspace/types/workspace-handle.types'
import { loadProxyConfig, type ProxyEnvironmentConfig } from '../config/proxy-config'

/**
 * PreviewOrchestratorService - Orchestrates preview server status and logs
 *
 * Responsibilities:
 * - Fetch preview server status from sandbox (via environment driver)
 * - Stream preview server logs
 *
 * Note: Start/stop/restart are now handled directly by the Claude Code agent
 * via /opt/moatless/bin/server preview in the sandbox container.
 */
@injectable()
export class PreviewOrchestratorService {
  private readonly proxyConfig: ProxyEnvironmentConfig

  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
  ) {
    this.proxyConfig = loadProxyConfig()
  }

  private async getWorkspaceHandleIfExists(taskId: string): Promise<WorkspaceHandle | null> {
    return await this.runtimeManager.getExistingWorkspaceHandle(taskId)
  }

  /**
   * Get preview server status
   */
  async getStatus(taskId: string): Promise<ServerStatus> {
    // Get repositoryId from task record to check preview config
    const task = await this.taskRepository.get(taskId)
    if (task?.repositoryId) {
      const repoConfig = await this.repoService.getRepositoryById(task.repositoryId)
      const hasPreviewConfig =
        repoConfig?.previewConfig?.serverStartCommands &&
        repoConfig.previewConfig.serverStartCommands.length > 0
      if (!hasPreviewConfig) {
        return {
          status: 'stopped',
          pid: undefined,
          url: undefined,
          uptimeSeconds: 0,
          dirtyContext: false,
        }
      }
    }

    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getPreviewServerStatus) {
      return {
        status: 'stopped',
        pid: undefined,
        url: undefined,
        uptimeSeconds: 0,
        dirtyContext: false,
      }
    }

    const status = await handle.driver.getPreviewServerStatus()
    const externalUrl =
      status.externalUrl ||
      (status.status === 'started' &&
      this.proxyConfig.useHostnameRouting &&
      this.proxyConfig.proxyDomain
        ? `${this.proxyConfig.proxyProtocol}://${taskId}.${this.proxyConfig.proxyDomain}`
        : undefined)

    return {
      status: status.status,
      pid: status.pid,
      url: externalUrl,
      uptimeSeconds: status.uptimeSeconds || 0,
      error: status.error,
      logs: status.logs,
      dirtyContext: false, // dirtyContext is ephemeral — sandbox tracks it
    }
  }

  /**
   * Get status for any server by name
   */
  async getServerStatus(
    taskId: string,
    serverName: string,
  ): Promise<{ status: string; url: string | undefined; error: string | undefined }> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)

    if (!handle?.driver.getServerStatus) {
      return { status: 'stopped', url: undefined, error: undefined }
    }

    try {
      const status = await handle.driver.getServerStatus(serverName)
      const externalUrl =
        status.externalUrl ||
        (status.status === 'started' &&
        this.proxyConfig.useHostnameRouting &&
        this.proxyConfig.proxyDomain
          ? `${this.proxyConfig.proxyProtocol}://${taskId}${serverName === 'preview' ? '' : `--${serverName}`}.${this.proxyConfig.proxyDomain}`
          : undefined)

      return {
        status: status.status,
        url: externalUrl,
        error: status.error,
      }
    } catch {
      return { status: 'stopped', url: undefined, error: undefined }
    }
  }

  /**
   * Get server logs by name
   */
  async getLogs(taskId: string, serverName: string, lines?: number): Promise<string> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)

    // Try generic multi-server method first
    if (handle?.driver.getServerLogs) {
      return await handle.driver.getServerLogs(serverName, lines || 100)
    }

    // Fallback to legacy preview-only method
    if (serverName === 'preview' && handle?.driver.getPreviewServerLogs) {
      return await handle.driver.getPreviewServerLogs(lines || 100)
    }

    return 'No server running'
  }

  /**
   * Stream server logs by name
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamLogs(
    taskId: string,
    serverName: string,
  ): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)

    // Try generic multi-server method first
    if (handle?.driver.streamServerLogs) {
      yield* handle.driver.streamServerLogs(serverName, 1000)
      return
    }

    // Fallback to legacy preview-only method
    if (serverName === 'preview' && handle?.driver.streamPreviewServerLogs) {
      yield* handle.driver.streamPreviewServerLogs(1000)
      return
    }

    yield {
      type: 'error',
      data: {
        error: 'Server is not running',
        timestamp: new Date().toISOString(),
      },
    }
  }
}
