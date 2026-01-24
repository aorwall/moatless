import type {
  FileTreeWithChangesResponse,
  GitChanges,
  GitStatus,
  ShellCommandLogEvent,
} from '@moatless/types'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import type { EventService } from '../../events/services/event.service'
import { loadProxyConfig } from '../../preview/config/proxy-config'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  FileSearchRequest,
  FileSearchResult,
  FileWithChangesResult,
  GitDiffResult,
  PreviewServerLogEvent,
  PreviewServerStatus,
  RunCommandOptions,
} from './base-environment-driver'
import { RemoteAgentProxy } from './remote-agent-proxy'

/**
 * Response from /command endpoint
 */
interface CommandResponse {
  exit_code: number
  stdout: string
  stderr: string
}

/**
 * Response from /read-file endpoint
 */
interface ReadFileResponse {
  content: string
  error?: string
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
 * Response from /write-file endpoint
 */
interface WriteFileResponse {
  success: boolean
  error?: string
}

/**
 * Response from /list-files endpoint
 */
interface ListFilesResponse {
  files: string[]
  error?: string
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
  getWorkerUrl: () => Promise<string>
  eventService: EventService
  getExternalPreviewUrl?: () => string | undefined
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
 * - Kubernetes: async () => `http://${podIp}:${workerPort}`
 *
 * Key responsibilities:
 * - Run commands on the sandbox via HTTP
 * - Create agent proxies that forward to the sandbox
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
   * Get the worker URL for this sandbox
   */
  protected getWorkerUrl(): Promise<string> {
    return this.config.getWorkerUrl()
  }

  /**
   * Get the worker base URL for preview proxying
   */
  async getWorkerBaseUrl(): Promise<string | undefined> {
    try {
      return await this.getWorkerUrl()
    } catch {
      return undefined
    }
  }

  protected async getWorkerProxyUrl(): Promise<string | undefined> {
    const workerBaseUrl = await this.getWorkerBaseUrl()
    return workerBaseUrl ? `${workerBaseUrl}/preview` : undefined
  }

  protected getExternalPreviewUrl(): string | undefined {
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
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, command, workerUrl },
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
      const response = await fetch(`${workerUrl}/command`, fetchOptions)

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
   * Create an agent that proxies to the sandbox
   */
  async createAgent(agentType: string, options?: AgentCreationOptions): Promise<BaseAgent> {
    const workerBaseUrl = await this.getWorkerUrl()

    this.driverLogger.info(
      { taskId: this.config.taskId, agentType, workerBaseUrl },
      'RemoteEnvironmentDriver: Creating remote agent proxy',
    )

    const agentOptions: Record<string, string> = {}
    if (options?.model) {
      agentOptions['model'] = options.model
    }
    if (options?.apiKey) {
      agentOptions['apiKey'] = options.apiKey
    }

    const agent = new RemoteAgentProxy(
      {
        workerBaseUrl,
        taskId: this.config.taskId,
        agentType,
        eventService: this.config.eventService,
      },
      agentOptions,
    )
    return agent
  }

  /**
   * Stop the agent on the sandbox
   */
  async stopAgent(): Promise<void> {
    let workerUrl: string
    try {
      workerUrl = await this.getWorkerUrl()
    } catch {
      this.driverLogger.warn(
        { taskId: this.config.taskId },
        'RemoteEnvironmentDriver: No worker URL to stop agent',
      )
      return
    }

    this.driverLogger.info(
      { taskId: this.config.taskId, workerUrl },
      'RemoteEnvironmentDriver: Stopping agent',
    )

    try {
      const response = await fetch(`${workerUrl}/stop-agent`, {
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

  /**
   * Read a file from the sandbox workspace
   */
  async readFile(filePath: string): Promise<string> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, filePath, workerUrl },
      'RemoteEnvironmentDriver: Reading file',
    )

    const response = await fetch(`${workerUrl}/read-file`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ filePath }),
      signal: AbortSignal.timeout(30000),
    })

    if (!response.ok) {
      const errorResult = (await response.json().catch(() => ({}))) as { error?: string }
      throw new Error(`Failed to read file: ${errorResult.error || response.statusText}`)
    }

    const result = (await response.json()) as ReadFileResponse
    return result.content
  }

  /**
   * Write a file to the sandbox workspace
   */
  async writeFile(filePath: string, content: string): Promise<void> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, filePath, workerUrl },
      'RemoteEnvironmentDriver: Writing file',
    )

    const response = await fetch(`${workerUrl}/write-file`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ filePath, content }),
      signal: AbortSignal.timeout(30000),
    })

    if (!response.ok) {
      const errorResult = (await response.json().catch(() => ({}))) as WriteFileResponse
      throw new Error(`Failed to write file: ${errorResult.error || response.statusText}`)
    }
  }

  /**
   * List files in the sandbox workspace
   */
  async listFiles(relativePath?: string, recursive?: boolean): Promise<string[]> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, relativePath, recursive, workerUrl },
      'RemoteEnvironmentDriver: Listing files',
    )

    const response = await fetch(`${workerUrl}/list-files`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ relativePath, recursive }),
      signal: AbortSignal.timeout(30000),
    })

    if (!response.ok) {
      const errorResult = (await response.json().catch(() => ({}))) as ListFilesResponse
      throw new Error(`Failed to list files: ${errorResult.error || response.statusText}`)
    }

    const result = (await response.json()) as ListFilesResponse
    return result.files
  }

  /**
   * Search for files in the sandbox workspace
   */
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResult> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, query: request.query, workerUrl },
      'RemoteEnvironmentDriver: Searching files',
    )

    const response = await fetch(`${workerUrl}/search-files`, {
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

  /**
   * Get git status for the sandbox workspace
   */
  async getGitStatus(): Promise<GitStatus> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, workerUrl },
      'RemoteEnvironmentDriver: Getting git status',
    )

    try {
      const response = await fetch(`${workerUrl}/git-status`, {
        signal: AbortSignal.timeout(10000),
      })

      if (!response.ok) {
        throw new Error(`Git status request failed: ${response.statusText}`)
      }

      return (await response.json()) as GitStatus
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to get git status',
      )
      return {
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  /**
   * Get git changes for the sandbox workspace
   */
  async getGitChanges(): Promise<GitChanges> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, workerUrl },
      'RemoteEnvironmentDriver: Getting git changes',
    )

    try {
      const response = await fetch(`${workerUrl}/git-changes`, {
        signal: AbortSignal.timeout(10000),
      })

      if (!response.ok) {
        throw new Error(`Git changes request failed: ${response.statusText}`)
      }

      return (await response.json()) as GitChanges
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to get git changes',
      )
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  // =============================================================================
  // Preview Server Methods - Read preview server state from sandbox
  // =============================================================================

  /**
   * Get preview server status from the sandbox
   */
  async getPreviewServerStatus(): Promise<PreviewServerStatus> {
    let workerUrl: string
    try {
      workerUrl = await this.getWorkerUrl()
    } catch {
      return {
        status: 'stopped',
        error: 'No worker URL available',
      }
    }

    const workerProxyUrl = await this.getWorkerProxyUrl()

    try {
      const response = await fetch(`${workerUrl}/server/status`, {
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        return {
          status: 'failed',
          error: `Status request failed: ${response.statusText}`,
        }
      }

      const status = (await response.json()) as ServerStatusResponse
      // Prioritize: driver-provided external URL > worker proxy > sandbox-provided URL
      const extUrl = this.getExternalPreviewUrl() || workerProxyUrl || status.externalUrl
      return {
        ...status,
        ...(workerProxyUrl && { internalUrl: workerProxyUrl }),
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
   * Get preview server logs from the sandbox
   */
  async getPreviewServerLogs(lines = 100): Promise<string> {
    let workerUrl: string
    try {
      workerUrl = await this.getWorkerUrl()
    } catch {
      return 'No worker URL available'
    }

    try {
      const response = await fetch(`${workerUrl}/server/logs?lines=${lines}`, {
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

  async *streamPreviewServerLogs(lines = 1000): AsyncGenerator<PreviewServerLogEvent> {
    try {
      const logs = await this.getPreviewServerLogs(lines)

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

  /**
   * Get file tree with git change information via worker
   */
  async getFileTreeWithChanges(rootPath = '.'): Promise<FileTreeWithChangesResponse> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, rootPath, workerUrl },
      'RemoteEnvironmentDriver: Getting file tree with changes',
    )

    try {
      const response = await fetch(`${workerUrl}/file-tree-with-changes`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ rootPath }),
        signal: AbortSignal.timeout(30000),
      })

      if (!response.ok) {
        const errorResult = (await response.json().catch(() => ({}))) as { error?: string }
        throw new Error(
          `Failed to get file tree with changes: ${errorResult.error || response.statusText}`,
        )
      }

      return (await response.json()) as FileTreeWithChangesResponse
    } catch (error) {
      this.driverLogger.error(
        { err: error as Error, taskId: this.config.taskId, rootPath },
        'RemoteEnvironmentDriver: Failed to get file tree with changes',
      )
      throw error
    }
  }

  /**
   * Get git diff via worker
   */
  async getGitDiff(): Promise<GitDiffResult> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, workerUrl },
      'RemoteEnvironmentDriver: Getting git diff',
    )

    try {
      const response = await fetch(`${workerUrl}/git-diff`, {
        signal: AbortSignal.timeout(30000),
      })

      if (!response.ok) {
        const errorResult = (await response.json().catch(() => ({}))) as { error?: string }
        throw new Error(`Failed to get git diff: ${errorResult.error || response.statusText}`)
      }

      return (await response.json()) as GitDiffResult
    } catch (error) {
      this.driverLogger.error(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to get git diff',
      )
      throw error
    }
  }

  /**
   * Read a file with git change information via worker
   */
  async readFileWithChanges(filePath: string): Promise<FileWithChangesResult> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, filePath, workerUrl },
      'RemoteEnvironmentDriver: Reading file with changes',
    )

    try {
      const response = await fetch(`${workerUrl}/read-file-with-changes`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ filePath }),
        signal: AbortSignal.timeout(30000),
      })

      if (!response.ok) {
        const errorResult = (await response.json().catch(() => ({}))) as { error?: string }
        throw new Error(
          `Failed to read file with changes: ${errorResult.error || response.statusText}`,
        )
      }

      return (await response.json()) as FileWithChangesResult
    } catch (error) {
      this.driverLogger.error(
        { err: error as Error, taskId: this.config.taskId, filePath },
        'RemoteEnvironmentDriver: Failed to read file with changes',
      )
      throw error
    }
  }

  /**
   * Get current commit hash via worker
   */
  async getCurrentCommitHash(): Promise<string | null> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, workerUrl },
      'RemoteEnvironmentDriver: Getting current commit hash',
    )

    try {
      const response = await fetch(`${workerUrl}/current-commit-hash`, {
        signal: AbortSignal.timeout(10000),
      })

      if (!response.ok) {
        this.driverLogger.warn(
          { taskId: this.config.taskId, status: response.status },
          'RemoteEnvironmentDriver: Failed to get commit hash',
        )
        return null
      }

      const result = (await response.json()) as { hash: string | null }
      return result.hash
    } catch (error) {
      this.driverLogger.warn(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to get commit hash',
      )
      return null
    }
  }

  // =============================================================================
  // Screenshot Methods - Capture screenshots via sandbox browser
  // =============================================================================

  /**
   * Capture screenshot of preview server via sandbox browser
   * @returns Screenshot result with base64 encoded image and URL
   */
  async captureScreenshot(): Promise<{ screenshotBase64: string; url: string }> {
    const workerUrl = await this.getWorkerUrl()

    this.driverLogger.debug(
      { taskId: this.config.taskId, workerUrl },
      'RemoteEnvironmentDriver: Capturing screenshot via sandbox',
    )

    try {
      const response = await fetch(`${workerUrl}/browser/screenshot`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        signal: AbortSignal.timeout(30000),
      })

      if (!response.ok) {
        const errorResult = (await response.json().catch(() => ({}))) as { error?: string }
        throw new Error(`Screenshot capture failed: ${errorResult.error || response.statusText}`)
      }

      const result = (await response.json()) as {
        success: boolean
        screenshotBase64?: string
        url?: string
        error?: string
      }

      if (!(result.success && result.screenshotBase64)) {
        throw new Error(result.error || 'Screenshot capture returned no data')
      }

      this.driverLogger.info(
        {
          taskId: this.config.taskId,
          url: result.url,
          base64Length: result.screenshotBase64.length,
        },
        'RemoteEnvironmentDriver: Screenshot captured successfully',
      )

      return {
        screenshotBase64: result.screenshotBase64,
        url: result.url || 'unknown',
      }
    } catch (error) {
      this.driverLogger.error(
        { err: error as Error, taskId: this.config.taskId },
        'RemoteEnvironmentDriver: Failed to capture screenshot',
      )
      throw error
    }
  }
}
