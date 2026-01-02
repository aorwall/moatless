import type { GitChanges, GitStatus, ShellCommandLogEvent } from '@moatless/types'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import type { EventService } from '../../events/services/event.service'
import type { KubernetesExecutionTarget } from '../types/workspace-runtime.types'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  FileSearchRequest,
  FileSearchResult,
  PreviewServerConfig,
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
  constructor(
    readonly taskId: string,
    private readonly target: KubernetesExecutionTarget,
    private readonly eventService: EventService,
  ) {}

  /**
   * Get the worker URL for this task's pod
   */
  private getWorkerUrl(): string {
    if (!this.target.podIp) {
      throw new Error(`Pod IP not available for task: ${this.taskId}`)
    }
    return `http://${this.target.podIp}:${this.target.workerPort}`
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
  getWorkerBaseUrl(): string | undefined {
    if (!this.target.podIp) {
      return undefined
    }
    return `http://${this.target.podIp}:${this.target.workerPort}`
  }

  /**
   * Run a command on the pod
   */
  async *runCommand(
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    const workerUrl = this.getWorkerUrl()

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
  createAgent(agentType: string, options?: AgentCreationOptions): BaseAgent {
    if (!this.target.podIp) {
      throw new Error(`Pod IP not available for task: ${this.taskId}`)
    }

    logger.info(
      { taskId: this.taskId, agentType, podIp: this.target.podIp },
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
        podIp: this.target.podIp,
        workerPort: this.target.workerPort,
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
    if (!this.target.podIp) {
      logger.warn({ taskId: this.taskId }, 'KubernetesEnvironmentDriver: No target to stop agent')
      return
    }

    const workerUrl = `http://${this.target.podIp}:${this.target.workerPort}`

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
    const workerUrl = this.getWorkerUrl()

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
    const workerUrl = this.getWorkerUrl()

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
    const workerUrl = this.getWorkerUrl()

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
    const workerUrl = this.getWorkerUrl()

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
    const workerUrl = this.getWorkerUrl()

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
    const workerUrl = this.getWorkerUrl()

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
  async startPreviewServer(config: PreviewServerConfig): Promise<PreviewServerStatus> {
    if (!this.target.podIp) {
      throw new Error(`No pod IP available for task: ${this.taskId}`)
    }

    const workerUrl = `http://${this.target.podIp}:${this.target.workerPort}`

    logger.info(
      { taskId: this.taskId, workerUrl },
      'KubernetesEnvironmentDriver: Starting preview server on pod',
    )

    const body: {
      serverCommand?: string
      envVars?: Record<string, string>
      setupCommands?: string[]
    } = {}
    if (config.serverCommand) {
      body.serverCommand = config.serverCommand
    }
    if (config.envVars) {
      body.envVars = config.envVars
    }
    if (config.setupCommands && config.setupCommands.length > 0) {
      body.setupCommands = config.setupCommands
    }

    const response = await fetch(`${workerUrl}/server/start`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(60000), // 60 second timeout for server start
    })

    if (!response.ok) {
      const errorData = (await response.json().catch(() => ({}))) as { error?: string }
      throw new Error(`Failed to start preview server: ${errorData.error || response.statusText}`)
    }

    const result = (await response.json()) as ServerStartResponse

    logger.info(
      { taskId: this.taskId, status: result.status, port: result.port },
      'KubernetesEnvironmentDriver: Preview server started on pod',
    )

    // Convert to PreviewServerStatus
    const status: PreviewServerStatus = {
      status: result.success ? 'started' : 'failed',
    }
    if (result.url) {
      status.internalUrl = result.url
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
    return status
  }

  /**
   * Stop preview server on the pod
   */
  async stopPreviewServer(): Promise<boolean> {
    if (!this.target.podIp) {
      logger.warn(
        { taskId: this.taskId },
        'KubernetesEnvironmentDriver: No target to stop preview server',
      )
      return true
    }

    const workerUrl = `http://${this.target.podIp}:${this.target.workerPort}`

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
    if (!this.target.podIp) {
      return {
        status: 'stopped',
        error: 'No pod IP available',
      }
    }

    const workerUrl = `http://${this.target.podIp}:${this.target.workerPort}`

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

      return (await response.json()) as ServerStatusResponse
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
    if (!this.target.podIp) {
      return 'No pod IP available'
    }

    const workerUrl = `http://${this.target.podIp}:${this.target.workerPort}`

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

  /**
   * Push changes to remote repository via worker pod
   */
  async pushChanges(options: {
    branch: string
    useUpstream?: boolean
  }): Promise<{ pushed: boolean; hasCommits: boolean; error?: string }> {
    const workerUrl = this.getWorkerUrl()

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
}
