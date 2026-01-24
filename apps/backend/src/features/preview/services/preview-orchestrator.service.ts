import type { ScreenshotClearResponse, ScreenshotListResponse, ServerStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { WorkspaceHandle } from '../../workspace/types/workspace-handle.types'
import { loadProxyConfig, type ProxyEnvironmentConfig } from '../config/proxy-config'

/**
 * PreviewOrchestratorService - Orchestrates preview server status and logs
 *
 * Responsibilities:
 * - Fetch preview server status from sandbox (via environment driver)
 * - Stream preview server logs
 * - Proxy browser screenshots
 *
 * Note: Start/stop/restart are now handled directly by the Claude Code agent
 * via /opt/moatless/bin/preview-server script in the sandbox container.
 */
@injectable()
export class PreviewOrchestratorService {
  private readonly serviceLogger: typeof logger
  private readonly proxyConfig: ProxyEnvironmentConfig

  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-orchestrator' })
    this.proxyConfig = loadProxyConfig()
  }

  private async getWorkspaceHandleIfExists(taskId: string): Promise<WorkspaceHandle | null> {
    return await this.runtimeManager.getExistingWorkspaceHandle(taskId)
  }

  /**
   * Get preview server status
   */
  async getStatus(taskId: string): Promise<ServerStatus> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)

    // Skip driver call if repository has no preview server configured
    if (snapshot?.repositoryId) {
      const repoConfig = await this.repoService.getRepositoryById(snapshot.repositoryId)
      const hasPreviewConfig =
        repoConfig?.previewConfig?.serverStartCommands &&
        repoConfig.previewConfig.serverStartCommands.length > 0
      if (!hasPreviewConfig) {
        return {
          status: 'stopped',
          pid: undefined,
          url: undefined,
          uptimeSeconds: 0,
          dirtyContext: snapshot?.preview?.dirtyContext,
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
        dirtyContext: snapshot?.preview?.dirtyContext,
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
      dirtyContext: snapshot?.preview?.dirtyContext,
    }
  }

  /**
   * Get preview server logs
   */
  async getLogs(taskId: string, lines?: number): Promise<string> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getPreviewServerLogs) {
      return 'No preview server running'
    }

    return await handle.driver.getPreviewServerLogs(lines || 100)
  }

  /**
   * Stream preview server logs
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.streamPreviewServerLogs) {
      yield {
        type: 'error',
        data: {
          error: 'Preview server is not running',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    yield* handle.driver.streamPreviewServerLogs(1000)
  }

  /**
   * Get browser screenshots for a task
   * Proxies to the sandbox's /browser/screenshots endpoint
   */
  async getBrowserScreenshots(taskId: string): Promise<ScreenshotListResponse> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getWorkerBaseUrl) {
      return { screenshots: [], total: 0 }
    }

    const workerUrl = await handle.driver.getWorkerBaseUrl()
    if (!workerUrl) {
      return { screenshots: [], total: 0 }
    }

    try {
      const response = await fetch(`${workerUrl}/browser/screenshots`)
      if (!response.ok) {
        this.serviceLogger.warn(
          { taskId, status: response.status },
          'Failed to fetch screenshots from sandbox',
        )
        return { screenshots: [], total: 0 }
      }
      return (await response.json()) as ScreenshotListResponse
    } catch (error) {
      this.serviceLogger.error(
        { err: error as Error, taskId },
        'Error fetching screenshots from sandbox',
      )
      return { screenshots: [], total: 0 }
    }
  }

  /**
   * Get a specific browser screenshot file
   * Proxies to the sandbox's /browser/screenshots/:filename endpoint
   */
  async getBrowserScreenshot(taskId: string, filename: string): Promise<Buffer | null> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getWorkerBaseUrl) {
      return null
    }

    const workerUrl = await handle.driver.getWorkerBaseUrl()
    if (!workerUrl) {
      return null
    }

    try {
      const response = await fetch(`${workerUrl}/browser/screenshots/${filename}`)
      if (!response.ok) {
        this.serviceLogger.warn(
          { taskId, filename, status: response.status },
          'Failed to fetch screenshot file from sandbox',
        )
        return null
      }
      return Buffer.from(await response.arrayBuffer())
    } catch (error) {
      this.serviceLogger.error(
        { err: error as Error, taskId, filename },
        'Error fetching screenshot file from sandbox',
      )
      return null
    }
  }

  /**
   * Clear all browser screenshots for a task
   * Proxies to the sandbox's DELETE /browser/screenshots endpoint
   */
  async clearBrowserScreenshots(taskId: string): Promise<ScreenshotClearResponse> {
    const handle = await this.getWorkspaceHandleIfExists(taskId)
    if (!handle?.driver.getWorkerBaseUrl) {
      return { success: false, deleted: 0 }
    }

    const workerUrl = await handle.driver.getWorkerBaseUrl()
    if (!workerUrl) {
      return { success: false, deleted: 0 }
    }

    try {
      const response = await fetch(`${workerUrl}/browser/screenshots`, {
        method: 'DELETE',
      })
      if (!response.ok) {
        this.serviceLogger.warn(
          { taskId, status: response.status },
          'Failed to clear screenshots from sandbox',
        )
        return { success: false, deleted: 0 }
      }
      return (await response.json()) as ScreenshotClearResponse
    } catch (error) {
      this.serviceLogger.error(
        { err: error as Error, taskId },
        'Error clearing screenshots from sandbox',
      )
      return { success: false, deleted: 0 }
    }
  }
}
