import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type { ShellCommandLogEvent } from '@moatless/types'
import { logger } from '../../../utils/logger'
import { loadProxyConfig } from '../../preview/config/proxy-config'
import type {
  BaseEnvironmentDriver,
  FileSearchRequest,
  FileSearchResult,
  PreviewServerLogEvent,
  PreviewServerStatus,
  RunCommandOptions,
} from './base-environment-driver'

/**
 * Response from /command endpoint
 */
interface CommandResponse {
  exit_code: number
  stdout: string
  stderr: string
}

/**
 * Response from /server/status endpoint
 */
interface ServerStatusResponse {
  status: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
  internalUrl?: string
  externalUrl?: string
  port?: number
  pid?: number
  uptimeSeconds?: number
  error?: string
  logs?: string
}

/**
 * Response from /server/logs endpoint
 */
interface ServerLogsResponse {
  logs: string
}

/**
 * Response from /search-files endpoint
 */
interface SearchFilesResponse {
  files: Array<{ path: string; matches: number }>
  totalMatches: number
  error?: string
}

/**
 * Configuration for the RemoteEnvironmentDriver
 */
export interface RemoteEnvironmentDriverConfig {
  taskId: string
  getSandboxUrl: () => Promise<string>
  getExternalPreviewUrl?: () => string | undefined
  getPreviewUrl?: () => Promise<string | undefined>
}

/**
 * RemoteEnvironmentDriver - Generic HTTP-based driver for remote sandboxes
 *
 * This driver forwards all operations via HTTP to a remote sandbox instance.
 * It can be used with both Docker containers and Kubernetes pods.
 *
 * The URL provider function is injected at construction time, allowing
 * different provisioners to provide their own URL resolution strategy:
 * - Docker: () => `http://localhost:${hostPort}`
 * - Kubernetes: async () => `http://${serviceName}.${namespace}.svc.cluster.local:${sandboxPort}`
 *
 * Key responsibilities:
 * - Run commands on the sandbox via HTTP
 * - Execute agents via HTTP (fire-and-forget, events via Redis)
 * - Read/write files on the sandbox workspace
 * - Manage preview servers on the sandbox
 */
export class RemoteEnvironmentDriver implements BaseEnvironmentDriver {
  protected readonly proxyConfig = loadProxyConfig()
  protected readonly driverLogger = logger.child({ driver: 'RemoteEnvironmentDriver' })

  constructor(protected readonly config: RemoteEnvironmentDriverConfig) {}

  get taskId(): string {
    return this.config.taskId
  }

  /**
   * Get the sandbox URL for this sandbox
   */
  protected getSandboxUrl(): Promise<string> {
    return this.config.getSandboxUrl()
  }

  /**
   * Get the sandbox base URL for preview proxying
   */
  async getSandboxBaseUrl(): Promise<string | undefined> {
    try {
      return await this.getSandboxUrl()
    } catch {
      return undefined
    }
  }

  protected async getSandboxProxyUrl(): Promise<string | undefined> {
    if (this.config.getPreviewUrl) {
      try {
        return await this.config.getPreviewUrl()
      } catch {
        return undefined
      }
    }
    const sandboxBaseUrl = await this.getSandboxBaseUrl()
    return sandboxBaseUrl ? `${sandboxBaseUrl}/preview` : undefined
  }

  public getExternalPreviewUrl(): string | undefined {
    if (this.config.getExternalPreviewUrl) {
      return this.config.getExternalPreviewUrl()
    }
    if (this.proxyConfig.useHostnameRouting && this.proxyConfig.proxyDomain) {
      return `${this.proxyConfig.proxyProtocol}://${this.config.taskId}.${this.proxyConfig.proxyDomain}`
    }
    return undefined
  }

  protected async getPreviewRuntime() {
    try {
      const { appContainer } = await import('../../../container/container')
      if (!appContainer) {
        this.driverLogger.warn(
          { taskId: this.config.taskId },
          'RemoteEnvironmentDriver: appContainer not available for preview runtime update',
        )
        return null
      }

      const previewModule = await import('../../preview/services/preview-runtime.service')
      return appContainer.get(previewModule.PreviewRuntimeService)
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to get PreviewRuntimeService - preview state updates will be skipped',
      )
      return null
    }
  }

  /**
   * Run a command on the sandbox
   */
  async *runCommand(
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    const sandboxUrl = await this.getSandboxUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, command, sandboxUrl },
      'RemoteEnvironmentDriver: Running command',
    )

    try {
      const fetchOptions: RequestInit = {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          command,
          timeout: options?.timeout,
          env: options?.env,
        }),
      }
      if (options?.timeout) {
        fetchOptions.signal = AbortSignal.timeout(options.timeout)
      }
      const response = await fetch(`${sandboxUrl}/command`, fetchOptions)

      const result = (await response.json()) as CommandResponse

      // Yield stdout if present
      if (result.stdout) {
        yield {
          type: 'stdout',
          text: result.stdout,
          timestamp: new Date().toISOString(),
        }
      }

      // Yield stderr if present
      if (result.stderr) {
        yield {
          type: 'stderr',
          text: result.stderr,
          timestamp: new Date().toISOString(),
        }
      }

      // Yield system message with exit code
      yield {
        type: 'system',
        text: `Command exited with code ${result.exit_code}`,
        timestamp: new Date().toISOString(),
      }

      if (result.exit_code !== 0) {
        this.driverLogger.warn(
          { taskId: this.config.taskId, command, exitCode: result.exit_code },
          'RemoteEnvironmentDriver: Command failed',
        )
      }
    } catch (error) {
      this.driverLogger.error(
        { err: error as Error, taskId: this.config.taskId, command },
        'RemoteEnvironmentDriver: Command execution failed',
      )

      yield {
        type: 'stderr',
        text: error instanceof Error ? error.message : 'Unknown error',
        timestamp: new Date().toISOString(),
      }
    }
  }

  /**
   * Execute an agent on the sandbox.
   * This is a fire-and-forget HTTP POST - events are handled via Redis subscription.
   */
  async executeAgent(request: AgentRunRequest): Promise<void> {
    const sandboxUrl = await this.getSandboxUrl()

    this.driverLogger.info(
      { taskId: this.config.taskId, sandboxUrl },
      'RemoteEnvironmentDriver: Executing agent on sandbox',
    )

    const response = await fetch(`${sandboxUrl}/execute-agent`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(request),
    })

    if (!response.ok) {
      const errorData = (await response.json().catch(() => ({}))) as { error?: string }
      throw new Error(
        `Failed to start agent on sandbox: ${response.status} ${errorData.error || response.statusText}`,
      )
    }

    const result = (await response.json()) as { success: boolean; error?: string }
    if (!result.success) {
      throw new Error(`Sandbox returned failure: ${result.error}`)
    }

    this.driverLogger.info(
      { taskId: this.config.taskId },
      'RemoteEnvironmentDriver: Agent execution started on sandbox',
    )
    // Fire-and-forget: Events handled by TaskEventHandler via Redis
  }

  /**
   * Stop the agent on the sandbox
   */
  async stopAgent(): Promise<void> {
    let sandboxUrl: string
    try {
      sandboxUrl = await this.getSandboxUrl()
    } catch {
      this.driverLogger.warn(
        { taskId: this.config.taskId },
        'RemoteEnvironmentDriver: No sandbox URL to stop agent',
      )
      return
    }

    this.driverLogger.info(
      { taskId: this.config.taskId, sandboxUrl },
      'RemoteEnvironmentDriver: Stopping agent',
    )

    try {
      const response = await fetch(`${sandboxUrl}/stop-agent`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId: this.config.taskId }),
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        this.driverLogger.warn(
          { taskId: this.config.taskId, status: response.status },
          'RemoteEnvironmentDriver: Stop agent request failed',
        )
      }
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to stop agent',
      )
    }
  }

  async notifyMessages(): Promise<void> {
    let sandboxUrl: string
    try {
      sandboxUrl = await this.getSandboxUrl()
    } catch {
      this.driverLogger.debug(
        { taskId: this.config.taskId },
        'RemoteEnvironmentDriver: No sandbox URL to notify messages',
      )
      return
    }

    try {
      const response = await fetch(`${sandboxUrl}/notify-messages`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
        signal: AbortSignal.timeout(3000),
      })

      if (!response.ok) {
        this.driverLogger.warn(
          { taskId: this.config.taskId, status: response.status },
          'RemoteEnvironmentDriver: Notify messages request failed',
        )
      }
    } catch (error) {
      this.driverLogger.debug(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to notify messages (best-effort)',
      )
    }
  }

  /**
   * Search for files in the sandbox workspace
   */
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResult> {
    const sandboxUrl = await this.getSandboxUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, query: request.query, sandboxUrl },
      'RemoteEnvironmentDriver: Searching files',
    )

    const response = await fetch(`${sandboxUrl}/search-files`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(request),
      signal: AbortSignal.timeout(30000),
    })

    if (!response.ok) {
      const errorResult = (await response.json().catch(() => ({}))) as SearchFilesResponse
      throw new Error(`Failed to search files: ${errorResult.error || response.statusText}`)
    }

    const result = (await response.json()) as SearchFilesResponse
    return {
      files: result.files,
      totalMatches: result.totalMatches,
    }
  }

  // =============================================================================
  // Preview Server Methods - Read preview server state from sandbox
  // =============================================================================

  /**
   * Get preview server status from the sandbox
   */
  async getPreviewServerStatus(): Promise<PreviewServerStatus> {
    let sandboxUrl: string
    try {
      sandboxUrl = await this.getSandboxUrl()
    } catch {
      return {
        status: 'stopped',
        error: 'No sandbox URL available',
      }
    }

    const sandboxProxyUrl = await this.getSandboxProxyUrl()

    try {
      const response = await fetch(`${sandboxUrl}/server/status`, {
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        return {
          status: 'failed',
          error: `Status request failed: ${response.statusText}`,
        }
      }

      const status = (await response.json()) as ServerStatusResponse
      // Prioritize: driver-provided external URL > sandbox proxy > sandbox-provided URL
      const extUrl = this.getExternalPreviewUrl() || sandboxProxyUrl || status.externalUrl
      return {
        ...status,
        ...(sandboxProxyUrl && { internalUrl: sandboxProxyUrl }),
        ...(extUrl && { externalUrl: extUrl }),
      }
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to get preview server status',
      )
      return {
        status: 'stopped',
        error: error instanceof Error ? error.message : 'Unknown error',
      }
    }
  }

  /**
   * Get status for a specific server by name from the sandbox
   */
  async getServerStatus(serverName: string): Promise<PreviewServerStatus> {
    let sandboxUrl: string
    try {
      sandboxUrl = await this.getSandboxUrl()
    } catch {
      return {
        status: 'stopped',
        error: 'No sandbox URL available',
      }
    }

    try {
      const response = await fetch(`${sandboxUrl}/servers/${serverName}/status`, {
        signal: AbortSignal.timeout(1000),
      })

      if (!response.ok) {
        return {
          status: 'failed',
          error: `Status request failed: ${response.statusText}`,
        }
      }

      return (await response.json()) as PreviewServerStatus
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId, serverName },
        'RemoteEnvironmentDriver: Failed to get server status',
      )
      return {
        status: 'stopped',
        error: error instanceof Error ? error.message : 'Unknown error',
      }
    }
  }

  /**
   * Get preview server logs from the sandbox
   */
  async getPreviewServerLogs(lines = 100): Promise<string> {
    let sandboxUrl: string
    try {
      sandboxUrl = await this.getSandboxUrl()
    } catch {
      return 'No sandbox URL available'
    }

    try {
      const response = await fetch(`${sandboxUrl}/server/logs?lines=${lines}`, {
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        return `Failed to get logs: ${response.statusText}`
      }

      const result = (await response.json()) as ServerLogsResponse
      return result.logs
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to get preview server logs',
      )
      return error instanceof Error ? error.message : 'Unknown error'
    }
  }

  // biome-ignore lint/suspicious/useAwait: Generator function delegates to async generator via yield*
  async *streamPreviewServerLogs(lines = 1000): AsyncGenerator<PreviewServerLogEvent> {
    yield* this.streamServerLogs('preview', lines)
  }

  // =============================================================================
  // Multi-Server Logs (Generic - for any server by name)
  // =============================================================================

  /**
   * Get logs for a specific server by name from the sandbox
   */
  async getServerLogs(serverName: string, lines = 100): Promise<string> {
    let sandboxUrl: string
    try {
      sandboxUrl = await this.getSandboxUrl()
    } catch {
      return 'No sandbox URL available'
    }

    try {
      const response = await fetch(`${sandboxUrl}/servers/${serverName}/logs?lines=${lines}`, {
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        // Fallback to legacy endpoint for backward compat with older sandboxes
        if (serverName === 'preview') {
          return this.getPreviewServerLogs(lines)
        }
        return `Failed to get logs: ${response.statusText}`
      }

      const result = (await response.json()) as ServerLogsResponse
      return result.logs
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId, serverName },
        'RemoteEnvironmentDriver: Failed to get server logs',
      )
      return error instanceof Error ? error.message : 'Unknown error'
    }
  }

  async *streamServerLogs(serverName: string, lines = 1000): AsyncGenerator<PreviewServerLogEvent> {
    try {
      const logs = await this.getServerLogs(serverName, lines)

      yield {
        type: 'reset',
        data: { timestamp: new Date().toISOString() },
      }

      if (logs) {
        yield {
          type: 'chunk',
          data: { text: logs, timestamp: new Date().toISOString() },
        }
      }
    } catch (error) {
      yield {
        type: 'error',
        data: {
          error: error instanceof Error ? error.message : 'Unknown error',
          timestamp: new Date().toISOString(),
        },
      }
    }
  }
}
