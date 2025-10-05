import { randomUUID } from 'node:crypto'
import type {
  EnvironmentStatus,
  RepositoryConfig,
  ServerStatus,
  ShellCommandLogEvent,
  ShellCommandResult,
} from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import type { EventService } from '../services/event.service.js'

export abstract class BaseEnvironment {
  protected currentAgent: BaseAgent | undefined

  constructor(
    protected readonly taskId: string,
    protected readonly repositoryConfig: RepositoryConfig,
    protected readonly eventService?: EventService,
  ) {}

  /**
   * Emit environment status change event
   */
  protected async emitStatusChange(
    status: EnvironmentStatus,
    previousStatus: EnvironmentStatus,
    error?: Error | string,
  ): Promise<void> {
    if (!this.eventService) {
      return
    }

    await this.eventService.emitEvent({
      taskId: this.taskId,
      type: 'environment',
      subType: status,
      payload: {
        status,
        previousStatus,
        workspaceId: this.taskId, // Using taskId as workspaceId for now
        error: error instanceof Error ? error.message : error,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
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
   * Stream server logs via SSE
   */
  abstract streamServerLogs?(): AsyncGenerator<{ type: string; data: any }>

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
  abstract runCommand(command: string, timeout?: number): Promise<ShellCommandResult>

  /**
   * Run a command with streaming output
   */
  abstract runCommandWithStream(
    command: string,
    timeout?: number,
  ): AsyncGenerator<ShellCommandLogEvent>

  /**
   * Check if an agent is currently running in this workspace
   */
  isRunning(): boolean {
    return !!this.currentAgent
  }

  /**
   * Run setup commands configured in the repository
   */
  abstract runSetupCommands(): Promise<void>

  /**
   * Set up the environment (starts the environment, then run setup commands)
   */
  abstract setup(): Promise<void>

  /**
   * Refresh repository configuration by re-running setup commands.
   * This stops the preview server (if running) and re-runs setup commands with latest config.
   * The agent continues running unaffected.
   */
  abstract refreshConfiguration(latestConfig: RepositoryConfig): Promise<void>

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
  // Build and Deploy Methods
  // ===============================

  /**
   * Build the project using the configured build command
   */
  abstract buildProject(): Promise<{
    success: boolean
    outputDirectory: string
    logs: string
    duration: number
  }>

  /**
   * Deploy the built project to Netlify
   */
  abstract deployToNetlify(
    siteName: string | undefined,
    authToken: string,
    existingSiteId?: string,
  ): Promise<{
    success: boolean
    siteId?: string
    deployUrl?: string
    siteUrl?: string
    deployId?: string
    logs: string
    duration: number
  }>
}
