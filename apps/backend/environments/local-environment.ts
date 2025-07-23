import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import { promisify } from 'node:util'
import type { CommandResult, RepositoryConfig, ServerStatus } from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import { ClaudeCodeAgent } from '../agents/claude-code/claude-code-agent.js'
import type { EventService } from '../services/event.service.js'
import { PreviewServer } from '../services/preview-server.js'
import { createTaskLogger } from '../utils/logger.js'
import { BaseEnvironment } from './base-environment.js'

const execAsync = promisify(exec)

export class LocalEnvironment extends BaseEnvironment {
  private setupInProgress = false
  private setupComplete = false
  private workspacePath = ''
  private _createdAt = new Date()
  private _error?: string
  private logger = createTaskLogger(this.taskId)
  private previewServer?: PreviewServer

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath?: string,
    eventService?: EventService,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.workspacePath = workspacePath || ''
    this.logger = createTaskLogger(taskId)

    this.logger.debug('LocalEnvironment constructor', {
      repositoryId: repositoryConfig.id,
      repositoryPath: repositoryConfig.path,
      codeExecutionType: repositoryConfig.codeExecution.type,
      workspacePath: this.workspacePath,
    })
  }

  getType(): 'local' {
    return 'local'
  }

  getStatus(): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) return 'error'
    if (this.setupInProgress) return 'initializing'
    if (this.setupComplete) return 'ready'
    return 'not_created'
  }

  getCreatedAt(): Date {
    return this._createdAt
  }

  getError(): string | undefined {
    return this._error
  }

  getWorkspacePath(): string {
    return this.workspacePath
  }

  async setup(iterationNumber?: number): Promise<void> {
    const previousStatus = this.getStatus()
    this.setupInProgress = true
    await this.emitStatusChange(this.getStatus(), previousStatus)

    const startTime = Date.now()

    this.logger.info('Starting LocalEnvironment setup', {
      repositoryId: this.repositoryConfig.id,
      workspacePath: this.workspacePath,
      iterationNumber,
    })

    try {
      if (!this.workspacePath) {
        throw new Error('Workspace path must be provided to LocalEnvironment')
      }

      // Check if workspace path exists
      if (!(await this.pathExists(this.workspacePath))) {
        throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
      }

      // Port allocation would be handled here if needed

      // Run setup commands in background
      this.runSetupCommandsInBackground(startTime).catch(async (error) => {
        const previousStatus = this.getStatus()
        this._error = error instanceof Error ? error.message : String(error)
        await this.emitStatusChange(
          this.getStatus(),
          previousStatus,
          error instanceof Error ? error : String(error),
        )
        this.logger.error('Background setup commands failed', error as Error)
      })

      this.logger.info('LocalEnvironment setup completed', {
        workspacePath: this.workspacePath,
        setupTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      const previousStatus = this.getStatus()
      this._error = error instanceof Error ? error.message : String(error)
      this.setupInProgress = false
      await this.emitStatusChange(
        this.getStatus(),
        previousStatus,
        error instanceof Error ? error : String(error),
      )

      this.logger.error('LocalEnvironment setup failed', error as Error, {
        setupTimeMs: failureTime,
        workspacePath: this.workspacePath,
      })

      throw error
    }
  }

  private async runSetupCommandsInBackground(startTime: number): Promise<void> {
    try {
      const setupCommands = this.repositoryConfig.codeExecution.setupCommands || []

      for (const command of setupCommands) {
        this.logger.info(`Running setup command: ${command}`)
        const result = await this.runCommand(command, 300000) // 5 minutes timeout

        if (result.exitCode !== 0) {
          this.logger.warn(`Setup command failed: ${command}`, {
            exitCode: result.exitCode,
            output: result.output,
          })
        }
      }

      const previousStatus = this.getStatus()
      this.setupComplete = true
      this.setupInProgress = false
      await this.emitStatusChange(this.getStatus(), previousStatus)

      this.logger.info('Setup commands completed successfully', {
        totalSetupTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      this.setupInProgress = false
      throw error
    }
  }

  async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
    this.logger.debug(`Running command: ${command}`)

    try {
      const { stdout } = await execAsync(command, {
        cwd: this.workspacePath,
        timeout,
        maxBuffer: 1024 * 1024, // 1MB buffer
      })

      return {
        exitCode: 0,
        output: stdout.trim(),
        durationMs: 0,
        command,
      }
    } catch (error) {
      const execError = error as Error & { code?: number; stdout?: string; stderr?: string }
      this.logger.error('Command execution failed', execError, {
        command,
        workspacePath: this.workspacePath,
      })

      return {
        exitCode: execError.code || 1,
        output: execError.stdout?.trim() || execError.message || '',
        durationMs: 0,
        command,
      }
    }
  }

  async startServer(): Promise<ServerStatus> {
    const serverCommand = this.repositoryConfig.previewConfig?.serverStartCommand
    if (!serverCommand) {
      throw new Error('No server command configured')
    }

    if (this.previewServer) {
      throw new Error('Server is already running')
    }

    this.logger.info('Starting preview server', { serverCommand })

    try {
      this.previewServer = new PreviewServer(
        this.taskId,
        this.workspacePath,
        serverCommand,
        this.repositoryConfig.codeExecution.envVars || {},
      )

      const result = await this.previewServer.start()

      return {
        running: result.running,
        configured: true,
        url: `http://localhost:${result.port}`,
        pid: result.pid,
        uptimeSeconds: result.uptime_seconds,
      }
    } catch (error) {
      this.previewServer = undefined
      throw error
    }
  }

  async stopServer(): Promise<boolean> {
    if (!this.previewServer) {
      return false
    }

    this.logger.info('Stopping preview server')

    try {
      const result = await this.previewServer.stop()
      this.previewServer = undefined
      return result
    } catch (error) {
      this.logger.error('Error stopping server', error as Error)
      this.previewServer = undefined
      return false
    }
  }

  async getServerStatus(): Promise<ServerStatus> {
    if (!this.previewServer) {
      const hasServerCommand = !!this.repositoryConfig.previewConfig?.serverStartCommand
      return {
        running: false,
        configured: hasServerCommand,
        uptimeSeconds: 0,
      }
    }

    const status = this.previewServer.getStatus(true)
    // For proxy approach, we don't expose the direct URL
    if (status.running) {
      status.url = undefined // Frontend will use proxy endpoint
    }
    return status
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (!this.previewServer) {
      return 'No server running'
    }

    return this.previewServer.getLogs(lines)
  }

  getPreviewPort(): number | undefined {
    return this.previewServer?.getPort()
  }

  async proxyPreviewRequest(request: {
    method: string
    path: string
    headers: Headers
    body?: string
  }): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }> {
    if (!this.previewServer) {
      throw new Error('Preview server not running')
    }

    return this.previewServer.proxyRequest(request)
  }

  async createAgent(agentType: string, options?: AgentOptions): Promise<BaseAgent> {
    this.logger.info('Creating agent', { agentType, taskId: this.taskId })

    if (agentType === 'claude-code') {
      return new ClaudeCodeAgent(this.taskId, options)
    }
    throw new Error(`Unknown agent type: ${agentType}`)
  }

  async *runAgent(
    agentType: string,
    message: string,
    options?: {
      workspaceDir?: string
      iterationNumber?: number
      resume?: string
      model?: string
    },
  ): AsyncGenerator<any> {
    this.logger.info('Running agent', { agentType, taskId: this.taskId })

    // Create agent options
    const agentOptions = options?.model ? { model: options.model } : {}

    // Create agent instance
    const agent = await this.createAgent(agentType, agentOptions)
    this.currentAgent = agent

    try {
      // Prepare run options
      const runOptions = {
        workspaceDir: options?.workspaceDir || this.workspacePath,
        iterationNumber: options?.iterationNumber,
        resume: options?.resume,
        model: options?.model,
      }

      // Run the agent and yield all events
      for await (const event of agent.run(message, runOptions)) {
        yield event
      }
    } finally {
      // Clear the current agent when done
      this.currentAgent = undefined
    }
  }

  async stopAgent(): Promise<boolean> {
    if (!this.currentAgent) {
      return false
    }

    this.logger.info('Stopping running agent')

    try {
      await this.currentAgent.stop()
      this.currentAgent = undefined
      return true
    } catch (error) {
      this.logger.error('Error stopping agent', error as Error)
      this.currentAgent = undefined
      return false
    }
  }

  async teardown(force = false): Promise<void> {
    this.logger.info('Tearing down LocalEnvironment', { force })

    try {
      // Stop agent
      await this.stopAgent()

      // Stop server
      await this.stopServer()

      // Port cleanup would be handled here if needed

      this.logger.info('LocalEnvironment teardown completed')
    } catch (error) {
      this.logger.error('Error during teardown', error as Error)
      if (!force) {
        throw error
      }
    }
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}
