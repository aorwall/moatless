import type { ShellCommandLogEvent } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import { EventService } from '../../events/services/event.service'
import type { KubernetesExecutionTarget } from '../types/workspace-runtime.types'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
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
 * KubernetesEnvironmentDriver - Environment driver for Kubernetes pods
 *
 * This driver implements BaseEnvironmentDriver by forwarding HTTP requests
 * to a worker pod running the worker-app.ts API.
 *
 * Key responsibilities:
 * - Run commands on the pod via HTTP
 * - Create agent proxies that forward to the pod
 * - Read files from the pod workspace
 */
@injectable()
export class KubernetesEnvironmentDriver implements BaseEnvironmentDriver {
  // Map of taskId -> execution target for looking up pod connection info
  private executionTargets = new Map<string, KubernetesExecutionTarget>()

  constructor(@inject(EventService) private readonly eventService: EventService) {}

  /**
   * Register an execution target for a task
   * Called by WorkspaceRuntimeManager when provisioning a Kubernetes workspace
   */
  registerTarget(taskId: string, target: KubernetesExecutionTarget): void {
    this.executionTargets.set(taskId, target)
    logger.debug(
      { taskId, podIp: target.podIp, podName: target.podName },
      'KubernetesEnvironmentDriver: Registered execution target',
    )
  }

  /**
   * Unregister an execution target
   * Called when workspace is cleaned up
   */
  unregisterTarget(taskId: string): void {
    this.executionTargets.delete(taskId)
  }

  /**
   * Get the worker URL for a task
   */
  private getWorkerUrl(taskId: string): string {
    const target = this.executionTargets.get(taskId)
    if (!target) {
      throw new Error(`No execution target registered for task: ${taskId}`)
    }
    if (!target.podIp) {
      throw new Error(`Pod IP not available for task: ${taskId}`)
    }
    return `http://${target.podIp}:${target.workerPort}`
  }

  /**
   * Get the execution target for a task
   */
  getTarget(taskId: string): KubernetesExecutionTarget | undefined {
    return this.executionTargets.get(taskId)
  }

  /**
   * Run a command on the pod
   */
  async *runCommand(
    workspacePath: string,
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    // Extract taskId from workspacePath (for Kubernetes, this should be /workspace)
    // We need to find the target by matching the path
    const taskId = this.findTaskIdByWorkspacePath(workspacePath)
    if (!taskId) {
      throw new Error(`No task found for workspace path: ${workspacePath}`)
    }

    const workerUrl = this.getWorkerUrl(taskId)

    logger.debug(
      { taskId, command, workspacePath, workerUrl },
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
          { taskId, command, exitCode: result.exit_code },
          'KubernetesEnvironmentDriver: Command failed',
        )
      }
    } catch (error) {
      logger.error(
        { err: error as Error, taskId, command },
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
  createAgent(taskId: string, agentType: string, options?: AgentCreationOptions): BaseAgent {
    const target = this.executionTargets.get(taskId)
    if (!target) {
      throw new Error(`No execution target registered for task: ${taskId}`)
    }
    if (!target.podIp) {
      throw new Error(`Pod IP not available for task: ${taskId}`)
    }

    logger.info(
      { taskId, agentType, podIp: target.podIp },
      'KubernetesEnvironmentDriver: Creating remote agent proxy',
    )

    const agentOptions: Record<string, string> = {}
    if (options?.model) {
      agentOptions['model'] = options.model
    }
    if (options?.apiKey) {
      agentOptions['apiKey'] = options.apiKey
    }

    return new RemoteAgentProxy(
      {
        podIp: target.podIp,
        workerPort: target.workerPort,
        taskId,
        agentType,
        eventService: this.eventService,
      },
      agentOptions,
    )
  }

  /**
   * Stop an agent on the pod
   */
  async stopAgent(taskId: string): Promise<void> {
    const target = this.executionTargets.get(taskId)
    if (!target?.podIp) {
      logger.warn({ taskId }, 'KubernetesEnvironmentDriver: No target to stop agent')
      return
    }

    const workerUrl = `http://${target.podIp}:${target.workerPort}`

    logger.info({ taskId, workerUrl }, 'KubernetesEnvironmentDriver: Stopping agent on pod')

    try {
      const response = await fetch(`${workerUrl}/stop-agent`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId }),
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        logger.warn(
          { taskId, status: response.status },
          'KubernetesEnvironmentDriver: Stop agent request failed',
        )
      }
    } catch (error) {
      logger.warn(
        { err: error as Error, taskId },
        'KubernetesEnvironmentDriver: Failed to stop agent on pod',
      )
    }
  }

  /**
   * Cleanup all agents by stopping them on all registered pods
   */
  async cleanup(): Promise<void> {
    logger.info(
      { registeredTargets: this.executionTargets.size },
      'KubernetesEnvironmentDriver: Cleaning up all agents',
    )

    const stopPromises: Promise<void>[] = []
    for (const [taskId] of this.executionTargets) {
      stopPromises.push(this.stopAgent(taskId))
    }

    await Promise.all(stopPromises)

    // Clear all registered targets
    this.executionTargets.clear()

    logger.info('KubernetesEnvironmentDriver: Cleanup complete')
  }

  /**
   * Read a file from the pod workspace
   */
  async readFile(workspacePath: string, filePath: string): Promise<string> {
    const taskId = this.findTaskIdByWorkspacePath(workspacePath)
    if (!taskId) {
      throw new Error(`No task found for workspace path: ${workspacePath}`)
    }

    const workerUrl = this.getWorkerUrl(taskId)

    logger.debug(
      { taskId, filePath, workerUrl },
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
   * Find taskId by workspace path
   * For Kubernetes, the workspace path is typically /workspace
   */
  private findTaskIdByWorkspacePath(workspacePath: string): string | undefined {
    // For Kubernetes pods, workspace is always /workspace
    // We need to track the taskId through context
    // This is a simplified implementation - in practice, the caller should pass taskId
    for (const [taskId] of this.executionTargets) {
      // Kubernetes pods use /workspace path
      if (workspacePath === '/workspace') {
        return taskId
      }
    }
    return undefined
  }

  // =============================================================================
  // Preview Server Methods - Forward preview server operations to pod
  // =============================================================================

  /**
   * Start preview server on the pod
   */
  async startPreviewServer(
    taskId: string,
    serverCommand?: string,
    envVars?: Record<string, string>,
  ): Promise<ServerStartResponse> {
    const target = this.executionTargets.get(taskId)
    if (!target?.podIp) {
      throw new Error(`No pod IP available for task: ${taskId}`)
    }

    const workerUrl = `http://${target.podIp}:${target.workerPort}`

    logger.info(
      { taskId, workerUrl },
      'KubernetesEnvironmentDriver: Starting preview server on pod',
    )

    const body: { serverCommand?: string; envVars?: Record<string, string> } = {}
    if (serverCommand) {
      body.serverCommand = serverCommand
    }
    if (envVars) {
      body.envVars = envVars
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
      { taskId, status: result.status, port: result.port },
      'KubernetesEnvironmentDriver: Preview server started on pod',
    )

    return result
  }

  /**
   * Stop preview server on the pod
   */
  async stopPreviewServer(taskId: string): Promise<boolean> {
    const target = this.executionTargets.get(taskId)
    if (!target?.podIp) {
      logger.warn({ taskId }, 'KubernetesEnvironmentDriver: No target to stop preview server')
      return true
    }

    const workerUrl = `http://${target.podIp}:${target.workerPort}`

    logger.info(
      { taskId, workerUrl },
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
          { taskId, status: response.status },
          'KubernetesEnvironmentDriver: Stop preview server request failed',
        )
        return false
      }

      const result = (await response.json()) as { success: boolean }
      return result.success
    } catch (error) {
      logger.warn(
        { err: error as Error, taskId },
        'KubernetesEnvironmentDriver: Failed to stop preview server on pod',
      )
      return false
    }
  }

  /**
   * Get preview server status from the pod
   */
  async getPreviewServerStatus(taskId: string): Promise<ServerStatusResponse> {
    const target = this.executionTargets.get(taskId)
    if (!target?.podIp) {
      return {
        status: 'stopped',
        error: 'No pod IP available',
      }
    }

    const workerUrl = `http://${target.podIp}:${target.workerPort}`

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
        { err: error as Error, taskId },
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
  async getPreviewServerLogs(taskId: string, lines = 100): Promise<string> {
    const target = this.executionTargets.get(taskId)
    if (!target?.podIp) {
      return 'No pod IP available'
    }

    const workerUrl = `http://${target.podIp}:${target.workerPort}`

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
        { err: error as Error, taskId },
        'KubernetesEnvironmentDriver: Failed to get preview server logs',
      )
      return error instanceof Error ? error.message : 'Unknown error'
    }
  }

  /**
   * Get the worker base URL for a task (for preview proxying)
   */
  getWorkerBaseUrl(taskId: string): string | undefined {
    const target = this.executionTargets.get(taskId)
    if (!target?.podIp) {
      return undefined
    }
    return `http://${target.podIp}:${target.workerPort}`
  }
}
