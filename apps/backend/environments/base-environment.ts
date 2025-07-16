import type { CommandResult, Event, RepositoryConfig, ServerStatus } from '@moatless/types'
import type { AgentOptions, AgentRunOptions, BaseAgent } from '../agents/base-agent.js'
import type { EventService } from '../services/index.js'

export abstract class BaseEnvironment {
  constructor(
    protected readonly taskId: string,
    protected readonly repositoryConfig: RepositoryConfig,
    protected readonly eventService: EventService,
  ) {}

  /**
   * Create an agent instance for execution
   */
  abstract createAgent(agentType: string, options?: AgentOptions): Promise<BaseAgent>

  /**
   * Execute an agent with a message
   */
  abstract executeAgent(
    agent: BaseAgent,
    message: string,
    runOptions: AgentRunOptions,
  ): AsyncGenerator<Event>

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
  abstract isRunning(): boolean

  /**
   * Set up the environment (starts the environment, then run setup commands)
   */
  abstract setup(iterationNumber?: number): Promise<void>

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
  abstract getStatus(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning'

  /**
   * Get any error message
   */
  abstract getError(): string | undefined

  /**
   * Get the creation timestamp
   */
  abstract getCreatedAt(): Date
}
