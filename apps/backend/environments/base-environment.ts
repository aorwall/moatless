import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import type { CommandResult, RepositoryConfig, ServerStatus } from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import type { EventService } from '../services/event.service.js'

export abstract class BaseEnvironment {
  protected currentAgent?: BaseAgent

  constructor(
    protected readonly taskId: string,
    protected readonly repositoryConfig: RepositoryConfig,
    protected readonly eventService?: EventService,
  ) {}

  /**
   * Emit environment status change event
   */
  protected async emitStatusChange(
    status: 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning',
    previousStatus: 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning',
    error?: Error | string,
  ): Promise<void> {
    if (!this.eventService) {
      return
    }

    await this.eventService.emitEvent({
      taskId: this.taskId,
      type: 'environment',
      subType: `environment:${status}`,
      payload: {
        status,
        previousStatus,
        workspaceId: this.taskId, // Using taskId as workspaceId for now
        error: error instanceof Error ? error.message : error,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: 'environment', // Required field for environment events
      isError: status === 'error',
    })
  }

  /**
   * Create an agent instance for execution
   */
  abstract createAgent(agentType: string, options?: AgentOptions): Promise<BaseAgent>

  /**
   * Run an agent with the given message and options
   * This replaces the separate createAgent + execute pattern
   */
  abstract runAgent(
    agentType: string,
    message: string,
    options?: {
      workspaceDir?: string
      iterationNumber?: number
      resume?: string
      model?: string
    },
  ): AsyncGenerator<any>

  /**
   * Get the current agent status
   */
  getAgentStatus(): {
    agentType?: string
    state: 'idle' | 'running' | 'none'
    sessionId?: string
  } {
    if (!this.currentAgent) {
      return { state: 'none' }
    }

    return {
      agentType: this.currentAgent.getAgentType(),
      state: 'running', // If we have a current agent, it's running
      sessionId: this.currentAgent.getSessionId(),
    }
  }

  /**
   * Stop the running agent for this workspace
   */
  abstract stopAgent(): Promise<boolean>

  /**
   * Start the server if configured
   */
  abstract startServer?(): Promise<ServerStatus>

  /**
   * Stop the server if running
   */
  abstract stopServer?(): Promise<boolean>

  /**
   * Get the current server status
   */
  abstract getServerStatus?(): Promise<ServerStatus>

  /**
   * Get server logs
   */
  abstract getServerLogs?(lines?: number): Promise<string>

  /**
   * Get the preview server port
   */
  abstract getPreviewPort?(): number | undefined

  /**
   * Proxy requests to the preview server
   */
  abstract proxyPreviewRequest?(request: {
    method: string
    path: string
    headers: Headers
    body?: string
  }): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }>

  /**
   * Run a command in the workspace
   */
  abstract runCommand(command: string, timeout?: number): Promise<CommandResult>

  /**
   * Check if an agent is currently running in this workspace
   */
  isRunning(): boolean {
    return !!this.currentAgent
  }

  /**
   * Set up the environment (starts the environment, then run setup commands)
   */
  abstract setup(iterationNumber?: number): Promise<void>

  /**
   * Ensures the environment is ready for operations.
   * If not ready, attempts to set it up.
   * @returns Promise that resolves when environment is ready
   */
  async ensureReady(): Promise<void> {
    const status = this.getStatus()

    // Fast path - already ready
    if (status === 'ready') {
      return
    }

    // Handle different states
    switch (status) {
      case 'not_created':
        await this.setup()
        break

      case 'error':
        // Allow environments to override for custom recovery
        await this.recoverFromError()
        break

      case 'initializing':
        // Wait for initialization to complete
        await this.waitForReady()
        break

      case 'cleaning':
        throw new Error('Environment is being cleaned up')

      default:
        throw new Error(`Unexpected environment status: ${status}`)
    }
  }

  /**
   * Override to implement custom error recovery
   */
  protected async recoverFromError(): Promise<void> {
    throw new Error('Environment is in error state and cannot recover')
  }

  /**
   * Override to implement waiting logic
   */
  protected async waitForReady(): Promise<void> {
    // Default: poll status until ready or timeout
    const maxWait = 60000 // 60 seconds
    const interval = 1000 // 1 second
    const startTime = Date.now()

    while (Date.now() - startTime < maxWait) {
      if (this.getStatus() === 'ready') {
        return
      }
      await new Promise((resolve) => setTimeout(resolve, interval))
    }

    throw new Error('Environment did not become ready within timeout')
  }

  /**
   * Stop and cleanup
   */
  abstract teardown(force?: boolean): Promise<void>

  /**
   * Get the workspace path
   */
  abstract getWorkspacePath(): string

  /**
   * Get the environment type
   */
  abstract getType(): 'local' | 'docker'

  /**
   * Get the environment status
   */
  abstract getStatus(): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning'

  /**
   * Get any error message
   */
  abstract getError(): string | undefined

  /**
   * Get the creation timestamp
   */
  abstract getCreatedAt(): Date

  // ===============================
  // Shared Utility Methods
  // ===============================

  /**
   * Check if a path exists (shared utility for both environment types)
   */
  protected async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  /**
   * Create environment variables object from configuration (shared utility)
   */
  protected createEnvironmentVariables(
    baseEnvVars: Record<string, string>,
    repositoryEnvVars?: Record<string, string>,
  ): Record<string, string> {
    return {
      ...baseEnvVars,
      ...(repositoryEnvVars || {}),
    }
  }

  /**
   * Format command string for safe execution (shared utility)
   */
  protected formatContainerCommand(command: string): string {
    // Escape double quotes in the command
    return command.replace(/"/g, '\\"')
  }

  /**
   * Handle background task execution with proper error handling (shared utility)
   */
  protected async handleBackgroundTasks(
    tasks: Array<() => Promise<void>>,
    onError?: (error: Error, taskIndex: number) => Promise<void>,
  ): Promise<void> {
    const promises = tasks.map(async (task, index) => {
      try {
        await task()
      } catch (error) {
        if (onError) {
          await onError(error as Error, index)
        } else {
          throw error
        }
      }
    })

    await Promise.all(promises)
  }

  /**
   * Validate repository configuration (shared utility)
   */
  protected validateRepositoryConfig(config: RepositoryConfig): void {
    if (!config.id) {
      throw new Error('Repository configuration must have an ID')
    }
    if (!config.path) {
      throw new Error('Repository configuration must have a path')
    }
    if (!config.codeExecution) {
      throw new Error('Repository configuration must have code execution settings')
    }
  }

  /**
   * Get safe timeout value with bounds checking (shared utility)
   */
  protected getSafeTimeout(timeout?: number, defaultTimeout = 30000, maxTimeout = 600000): number {
    if (!timeout) return defaultTimeout
    return Math.min(Math.max(timeout, 1000), maxTimeout) // Min 1s, max 10min
  }
}
