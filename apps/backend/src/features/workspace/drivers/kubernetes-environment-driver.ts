import type {
  FileTreeWithChangesResponse,
  GitChanges,
  GitStatus,
  ShellCommandLogEvent,
} from '@moatless/types'
import type { KubernetesService } from '../../../services/kubernetes.service'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import type { EventService } from '../../events/services/event.service'
import { loadProxyConfig } from '../../preview/config/proxy-config'
import type { KubernetesExecutionTarget } from '../types/workspace-runtime.types'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  FileSearchRequest,
  FileSearchResult,
  FileWithChangesResult,
  GitDiffResult,
  PreviewServerConfig,
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
 * Response from /server/start endpoint
 */
interface ServerStartResponse {
  success: boolean
  status?: string
  port?: number
  pid?: number
  url?: string
  uptimeSeconds?: number
  error?: string
}

/**
 * Response from /server/status endpoint
 */
interface ServerStatusResponse {
  status: 'stopped' | 'starting' | 'started' | 'failed'
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
 * KubernetesEnvironmentDriver - Task-scoped driver for Kubernetes pods
 *
 * Each instance is scoped to a single task with:
 * - taskId: The task this driver operates on
 * - target: The Kubernetes execution target (pod info)
 * - eventService: For forwarding agent events
 *
 * The driver forwards HTTP requests to a worker pod running the worker-app.ts API.
 *
 * Key responsibilities:
 * - Run commands on the pod via HTTP
 * - Create agent proxies that forward to the pod
 * - Read/write files on the pod workspace
 * - Manage preview servers on the pod
 */
export class KubernetesEnvironmentDriver implements BaseEnvironmentDriver {
  private readonly proxyConfig = loadProxyConfig()

  constructor(
    readonly taskId: string,
    private readonly target: KubernetesExecutionTarget,
    private readonly eventService: EventService,
    private readonly kubernetesService: KubernetesService,
  ) {}

  /**
   * Get the worker URL for this task's pod by querying Kubernetes
   * This fetches the pod IP on-demand to always get the current value
   */
  private async getWorkerUrl(): Promise<string> {
    const podIp = await this.kubernetesService.getPodIp(this.taskId)
    if (!podIp) {
      throw new Error(`Pod IP not available for task: ${this.taskId}`)
    }
    return `http://${podIp}:${this.target.workerPort}`
  }

  /**
   * Get the execution target for this driver
   */
  getTarget(): KubernetesExecutionTarget {
    return this.target
  }

  /**
   * Get the worker base URL for preview proxying
   */
  async getWorkerBaseUrl(): Promise<string | undefined> {
    const podIp = await this.kubernetesService.getPodIp(this.taskId)
    if (!podIp) {
      return undefined
    }
    return `http://${podIp}:${this.target.workerPort}`
  }

  private async getWorkerProxyUrl(): Promise<string | undefined> {
    const workerBaseUrl = await this.getWorkerBaseUrl()
    return workerBaseUrl ? `${workerBaseUrl}/server/proxy` : undefined
  }

  private getExternalPreviewUrl(): string | undefined {
    if (this.proxyConfig.useHostnameRouting && this.proxyConfig.proxyDomain) {
      return `${this.proxyConfig.proxyProtocol}://${this.taskId}.${this.proxyConfig.proxyDomain}`
    }
    return undefined
  }

  private async getPreviewRuntime() {
    try {
      const { appContainer } = await import('../../../container/container')
      if (!appContainer) {
        return null
      }

      const previewModule = await import('../../preview/services/preview-runtime.service')
      return appContainer.get(previewModule.PreviewRuntimeService)
    } catch (error) {
      logger.debug(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Preview runtime not available',
      )
      return null
    }
  }

  /**
   * Run a command on the pod
   */
  async *runCommand(
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, command, workerUrl },
      'KubernetesEnvironmentDriver: Running command on pod',
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
        logger.warn(
          { taskId: this.taskId, command, exitCode: result.exit_code },
          'KubernetesEnvironmentDriver: Command failed',
        )
      }
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: this.taskId, command },
        'KubernetesEnvironmentDriver: Command execution failed',
      )

      yield {
        type: 'stderr',
        text: error instanceof Error ? error.message : 'Unknown error',
        timestamp: new Date().toISOString(),
      }
    }
  }

  /**
   * Create an agent that proxies to the pod
   */
  async createAgent(agentType: string, options?: AgentCreationOptions): Promise<BaseAgent> {
    // Fetch pod IP on-demand from Kubernetes
    const workerBaseUrl = await this.getWorkerUrl()

    logger.info(
      { taskId: this.taskId, agentType, workerBaseUrl },
      'KubernetesEnvironmentDriver: Creating remote agent proxy',
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
        taskId: this.taskId,
        agentType,
        eventService: this.eventService,
      },
      agentOptions,
    )
    return agent
  }

  /**
   * Stop the agent on the pod
   */
  async stopAgent(): Promise<void> {
    const podIp = await this.kubernetesService.getPodIp(this.taskId)
    if (!podIp) {
      logger.warn({ taskId: this.taskId }, 'KubernetesEnvironmentDriver: No pod IP to stop agent')
      return
    }

    const workerUrl = `http://${podIp}:${this.target.workerPort}`

    logger.info(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Stopping agent on pod',
    )

    try {
      const response = await fetch(`${workerUrl}/stop-agent`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId: this.taskId }),
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        logger.warn(
          { taskId: this.taskId, status: response.status },
          'KubernetesEnvironmentDriver: Stop agent request failed',
        )
      }
    } catch (error) {
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to stop agent on pod',
      )
    }
  }

  /**
   * Read a file from the pod workspace
   */
  async readFile(filePath: string): Promise<string> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, filePath, workerUrl },
      'KubernetesEnvironmentDriver: Reading file from pod',
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
   * Write a file to the pod workspace
   */
  async writeFile(filePath: string, content: string): Promise<void> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, filePath, workerUrl },
      'KubernetesEnvironmentDriver: Writing file to pod',
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
   * List files in the pod workspace
   */
  async listFiles(relativePath?: string, recursive?: boolean): Promise<string[]> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, relativePath, recursive, workerUrl },
      'KubernetesEnvironmentDriver: Listing files on pod',
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
   * Search for files in the pod workspace
   */
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResult> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, query: request.query, workerUrl },
      'KubernetesEnvironmentDriver: Searching files on pod',
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
   * Get git status for the pod workspace
   */
  async getGitStatus(): Promise<GitStatus> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Getting git status from pod',
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
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to get git status',
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
   * Get git changes for the pod workspace
   */
  async getGitChanges(): Promise<GitChanges> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Getting git changes from pod',
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
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to get git changes',
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
  // Preview Server Methods - Forward preview server operations to pod
  // =============================================================================

  /**
   * Start preview server on the pod
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex logic needed for preview server initialization and error handling
  async startPreviewServer(config: PreviewServerConfig): Promise<PreviewServerStatus> {
    const workerUrl = await this.getWorkerUrl()

    logger.info(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Starting preview server on pod',
    )

    if (!config.serverCommands || config.serverCommands.length === 0) {
      throw new Error('Preview server requires at least one server command')
    }

    const body: {
      serverCommands: string[]
      envVars?: Record<string, string>
      setupCommands?: string[]
      startupTimeoutSeconds?: number
    } = {
      serverCommands: config.serverCommands,
    }
    if (config.envVars) {
      body.envVars = config.envVars
    }
    if (config.setupCommands && config.setupCommands.length > 0) {
      body.setupCommands = config.setupCommands
    }
    if (config.startupTimeoutSeconds) {
      body.startupTimeoutSeconds = config.startupTimeoutSeconds
    }

    const previewRuntime = await this.getPreviewRuntime()
    if (previewRuntime) {
      await previewRuntime.update(this.taskId, {
        status: 'starting',
        inspectorEnabled: config.enableInspector,
      })
      await previewRuntime.onStatusChange?.(this.taskId, { status: 'starting' })
    }

    const response = await fetch(`${workerUrl}/server/start`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(60000), // 60 second timeout for server start
    })

    if (!response.ok) {
      const errorData = (await response.json().catch(() => ({}))) as { error?: string }
      if (previewRuntime) {
        const errorMessage = errorData.error || response.statusText
        await previewRuntime.update(this.taskId, { status: 'failed', error: errorMessage })
        await previewRuntime.onStatusChange?.(this.taskId, {
          status: 'failed',
          error: errorMessage,
        })
      }
      throw new Error(`Failed to start preview server: ${errorData.error || response.statusText}`)
    }

    const result = (await response.json()) as ServerStartResponse

    logger.info(
      { taskId: this.taskId, status: result.status, port: result.port },
      'KubernetesEnvironmentDriver: Preview server started on pod',
    )

    const workerProxyUrl = `${workerUrl}/server/proxy`
    const externalUrl = this.getExternalPreviewUrl()

    // Convert to PreviewServerStatus
    const status: PreviewServerStatus = {
      status: result.success ? 'started' : 'failed',
      internalUrl: workerProxyUrl,
      externalUrl,
    }
    if (result.port !== undefined) {
      status.port = result.port
    }
    if (result.pid !== undefined) {
      status.pid = result.pid
    }
    if (result.uptimeSeconds !== undefined) {
      status.uptimeSeconds = result.uptimeSeconds
    }
    if (result.error) {
      status.error = result.error
    }

    if (previewRuntime) {
      if (status.status === 'started') {
        await previewRuntime.update(this.taskId, {
          status: 'started',
          port: status.port,
          internalUrl: workerProxyUrl,
          externalUrl: workerProxyUrl,
          pid: status.pid,
        })
        await previewRuntime.onStatusChange?.(this.taskId, {
          status: 'started',
          port: status.port,
          pid: status.pid,
          internalUrl: workerProxyUrl,
          externalUrl,
          uptimeSeconds: status.uptimeSeconds,
        })
      } else {
        await previewRuntime.update(this.taskId, {
          status: 'failed',
          error: status.error,
        })
        await previewRuntime.onStatusChange?.(this.taskId, {
          status: 'failed',
          error: status.error,
        })
      }
    }

    return status
  }

  /**
   * Stop preview server on the pod
   */
  async stopPreviewServer(): Promise<boolean> {
    const podIp = await this.kubernetesService.getPodIp(this.taskId)
    if (!podIp) {
      logger.warn(
        { taskId: this.taskId },
        'KubernetesEnvironmentDriver: No pod IP to stop preview server',
      )
      return true
    }

    const workerUrl = `http://${podIp}:${this.target.workerPort}`

    logger.info(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Stopping preview server on pod',
    )

    try {
      const response = await fetch(`${workerUrl}/server/stop`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        signal: AbortSignal.timeout(10000),
      })

      if (!response.ok) {
        logger.warn(
          { taskId: this.taskId, status: response.status },
          'KubernetesEnvironmentDriver: Stop preview server request failed',
        )
        return false
      }

      const result = (await response.json()) as { success: boolean }
      if (result.success) {
        const previewRuntime = await this.getPreviewRuntime()
        if (previewRuntime) {
          await previewRuntime.remove(this.taskId)
          await previewRuntime.onStatusChange?.(this.taskId, { status: 'stopped' })
        }
      }
      return result.success
    } catch (error) {
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to stop preview server on pod',
      )
      return false
    }
  }

  /**
   * Get preview server status from the pod
   */
  async getPreviewServerStatus(): Promise<PreviewServerStatus> {
    const podIp = await this.kubernetesService.getPodIp(this.taskId)
    if (!podIp) {
      return {
        status: 'stopped',
        error: 'No pod IP available',
      }
    }

    const workerUrl = `http://${podIp}:${this.target.workerPort}`
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
      return {
        ...status,
        internalUrl: workerProxyUrl,
        externalUrl: status.externalUrl || this.getExternalPreviewUrl(),
      }
    } catch (error) {
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to get preview server status',
      )
      return {
        status: 'stopped',
        error: error instanceof Error ? error.message : 'Unknown error',
      }
    }
  }

  /**
   * Get preview server logs from the pod
   */
  async getPreviewServerLogs(lines = 100): Promise<string> {
    const podIp = await this.kubernetesService.getPodIp(this.taskId)
    if (!podIp) {
      return 'No pod IP available'
    }

    const workerUrl = `http://${podIp}:${this.target.workerPort}`

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
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to get preview server logs',
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
   * Push changes to remote repository via worker pod
   */
  async pushChanges(options: {
    branch: string
    useUpstream?: boolean
  }): Promise<{ pushed: boolean; hasCommits: boolean; error?: string }> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, branch: options.branch, useUpstream: options.useUpstream, workerUrl },
      'KubernetesEnvironmentDriver: Pushing changes via pod',
    )

    try {
      const response = await fetch(`${workerUrl}/git-push`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          branch: options.branch,
          useUpstream: options.useUpstream,
        }),
        signal: AbortSignal.timeout(60000), // 60 second timeout for push
      })

      if (!response.ok) {
        const errorResult = (await response.json().catch(() => ({}))) as { error?: string }
        return {
          pushed: false,
          hasCommits: false,
          error: errorResult.error || response.statusText,
        }
      }

      const result = (await response.json()) as {
        pushed: boolean
        hasCommits: boolean
        error?: string
      }
      return result
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: this.taskId, branch: options.branch },
        'KubernetesEnvironmentDriver: Failed to push changes',
      )
      return {
        pushed: false,
        hasCommits: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      }
    }
  }

  /**
   * Create a git checkpoint via worker pod
   */
  async createGitCheckpoint(
    message: string,
  ): Promise<{ commitHash: string | null; hasChanges: boolean; error?: string }> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, message, workerUrl },
      'KubernetesEnvironmentDriver: Creating git checkpoint via pod',
    )

    try {
      const response = await fetch(`${workerUrl}/git-checkpoint`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message }),
        signal: AbortSignal.timeout(30000), // 30 second timeout
      })

      if (!response.ok) {
        const errorResult = (await response.json().catch(() => ({}))) as { error?: string }
        return {
          commitHash: null,
          hasChanges: false,
          error: errorResult.error || response.statusText,
        }
      }

      const result = (await response.json()) as {
        commitHash: string | null
        hasChanges: boolean
        error?: string
      }
      return result
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: this.taskId, message },
        'KubernetesEnvironmentDriver: Failed to create git checkpoint',
      )
      return {
        commitHash: null,
        hasChanges: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      }
    }
  }

  /**
   * Get file tree with git change information via worker pod
   */
  async getFileTreeWithChanges(rootPath = '.'): Promise<FileTreeWithChangesResponse> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, rootPath, workerUrl },
      'KubernetesEnvironmentDriver: Getting file tree with changes from pod',
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
      logger.error(
        { err: error as Error, taskId: this.taskId, rootPath },
        'KubernetesEnvironmentDriver: Failed to get file tree with changes',
      )
      throw error
    }
  }

  /**
   * Get git diff via worker pod
   */
  async getGitDiff(): Promise<GitDiffResult> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Getting git diff from pod',
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
      logger.error(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to get git diff',
      )
      throw error
    }
  }

  /**
   * Read a file with git change information via worker pod
   */
  async readFileWithChanges(filePath: string): Promise<FileWithChangesResult> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, filePath, workerUrl },
      'KubernetesEnvironmentDriver: Reading file with changes from pod',
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
      logger.error(
        { err: error as Error, taskId: this.taskId, filePath },
        'KubernetesEnvironmentDriver: Failed to read file with changes',
      )
      throw error
    }
  }

  /**
   * Get current commit hash via worker pod
   */
  async getCurrentCommitHash(): Promise<string | null> {
    const workerUrl = await this.getWorkerUrl()

    logger.debug(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Getting current commit hash from pod',
    )

    try {
      const response = await fetch(`${workerUrl}/current-commit-hash`, {
        signal: AbortSignal.timeout(10000),
      })

      if (!response.ok) {
        logger.warn(
          { taskId: this.taskId, status: response.status },
          'KubernetesEnvironmentDriver: Failed to get commit hash',
        )
        return null
      }

      const result = (await response.json()) as { hash: string | null }
      return result.hash
    } catch (error) {
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'KubernetesEnvironmentDriver: Failed to get commit hash',
      )
      return null
    }
  }
}
