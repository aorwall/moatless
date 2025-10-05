import { randomUUID } from 'node:crypto'
import path from 'node:path'
import type {
  RepositoryConfig,
  ServerStatus,
  ShellCommandLogEvent,
  ShellCommandResult,
} from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import { ClaudeCodeAgent } from '../agents/claude-code/claude-code-agent.js'
import type { EventService } from '../services/event.service.js'
import { PreviewServer } from '../services/preview-server.js'
import { ShellCommand } from '../services/shell-command.js'
import { pathExists } from '../utils/filesystem.js'
import { createTaskLogger } from '../utils/logger.js'
import { clearTaskLog } from '../utils/task-log.js'
import { BaseEnvironment } from './base-environment.js'

export class LocalEnvironment extends BaseEnvironment {
  private setupInProgress = false
  private setupComplete = false
  private setupCommandsExecuted = false
  private workspacePath = ''
  private _createdAt = new Date()
  private _error: string | undefined
  private logger = createTaskLogger(this.taskId)
  private previewServer: PreviewServer | undefined
  private shellCommands: Map<string, ShellCommand> = new Map()
  private commandCounter = 0
  private logFile: string | undefined

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath?: string,
    eventService?: EventService,
    logFile?: string,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.workspacePath = workspacePath || ''
    this.logFile = logFile
    this.logger = createTaskLogger(taskId)

    this.logger.debug('LocalEnvironment constructor', {
      repositoryId: repositoryConfig.id,
      repositoryPath: repositoryConfig.path,
      codeExecutionType: repositoryConfig.codeExecution.type,
      workspacePath: this.workspacePath,
    })

    // DO NOT try to recover existing servers - it's unsafe and creates fake process objects
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

  areSetupCommandsExecuted(): boolean {
    return this.setupCommandsExecuted
  }

  /**
   * Emit server event during setup commands
   */
  private async emitServerEvent(status: 'installing' | 'failed', error?: string): Promise<void> {
    if (!this.eventService) return

    try {
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'server' as const,
        subType: status,
        taskId: this.taskId,
        timestamp: new Date(),
        isError: status === 'failed',
        payload: {
          status,
          error,
        },
      })

      this.logger.debug('Server event emitted during setup', { status, taskId: this.taskId })
    } catch (eventError) {
      this.logger.error('Failed to emit server event during setup', eventError as Error)
    }
  }

  async runSetupCommands(): Promise<void> {
    const setupCommands = this.repositoryConfig.codeExecution.setupCommands || []

    if (setupCommands.length === 0) {
      this.logger.info('No setup commands configured, marking as executed')
      this.setupCommandsExecuted = true
      return
    }

    if (this.setupCommandsExecuted) {
      this.logger.info('Setup commands already executed, skipping')
      return
    }

    this.logger.info(`Running ${setupCommands.length} setup commands`, { setupCommands })

    // Check if preview server is configured
    const hasPreviewConfig = this.repositoryConfig.previewConfig?.serverStartCommands?.length

    // Emit 'installing' status if preview is configured
    if (hasPreviewConfig && this.eventService) {
      await this.emitServerEvent('installing')
    }

    try {
      for (const command of setupCommands) {
        this.logger.info(`Running setup command: ${command}`)
        const commandStartTime = Date.now()

        // Create shell command with log file
        const commandId = this.generateCommandId()
        const shellCommand = new ShellCommand()
        this.shellCommands.set(commandId, shellCommand)

        const result = await shellCommand.executeWithResult(command, {
          workingDirectory: this.workspacePath,
          env: this.repositoryConfig.codeExecution.envVars,
          timeout: 300000, // 5 minutes timeout
          logFile: this.logFile,
        })

        // Clean up completed command
        this.shellCommands.delete(commandId)

        const commandDuration = Date.now() - commandStartTime

        this.logger.info('Setup command completed', {
          command,
          exitCode: result.exitCode,
          durationMs: commandDuration,
          outputLength: result.output.length,
          output: result.output.slice(0, 500) + (result.output.length > 500 ? '...' : ''), // First 500 chars
        })

        if (result.exitCode !== 0) {
          // Emit 'failed' status if preview is configured
          if (hasPreviewConfig && this.eventService) {
            await this.emitServerEvent('failed', `Setup command failed: ${command}`)
          }

          throw new Error(
            `Setup command failed: ${command}. Exit code: ${result.exitCode}. Output: ${result.output}`,
          )
        }
      }

      this.setupCommandsExecuted = true
      this.logger.info('All setup commands completed successfully')
    } catch (error) {
      // Emit 'failed' status if preview is configured and not already emitted
      if (hasPreviewConfig && this.eventService) {
        const errorMsg = error instanceof Error ? error.message : String(error)
        await this.emitServerEvent('failed', errorMsg)
      }
      throw error
    }
  }

  async setup(): Promise<void> {
    const previousStatus = this.getStatus()
    this.setupInProgress = true
    await this.emitStatusChange(this.getStatus(), previousStatus)

    const startTime = Date.now()

    this.logger.info('Starting LocalEnvironment setup', {
      repositoryId: this.repositoryConfig.id,
      workspacePath: this.workspacePath,
    })

    try {
      if (!this.workspacePath) {
        throw new Error('Workspace path must be provided to LocalEnvironment')
      }

      // Check if workspace path exists
      if (!(await pathExists(this.workspacePath))) {
        throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
      }

      // Port allocation would be handled here if needed

      // Mark setup as complete
      this.setupComplete = true
      this.setupInProgress = false
      const previousStatus = 'initializing'
      await this.emitStatusChange(this.getStatus(), previousStatus)

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

  async refreshConfiguration(latestConfig: RepositoryConfig): Promise<void> {
    this.logger.info('Refreshing repository configuration', {
      taskId: this.taskId,
      oldConfigUpdatedAt: this.repositoryConfig.updatedAt,
      newConfigUpdatedAt: latestConfig.updatedAt,
    })

    const startTime = Date.now()

    try {
      // Step 1: Stop preview server if running
      if (this.previewServer) {
        this.logger.info('Stopping preview server before configuration refresh')
        await this.stopServer()
      }
      // Step 2: Update internal config reference
      ;(this as any).repositoryConfig = latestConfig // Cast to bypass readonly

      const totalDuration = Date.now() - startTime
      this.logger.info('Configuration refresh completed successfully', {
        durationMs: totalDuration,
      })
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.logger.error('Failed to refresh configuration', error as Error, {
        errorMessage,
        durationMs: Date.now() - startTime,
      })
      throw new Error(`Configuration refresh failed: ${errorMessage}`)
    }
  }

  async runCommand(
    command: string,
    timeout = 30000,
    additionalEnv?: Record<string, string>,
  ): Promise<ShellCommandResult> {
    this.logger.info('Starting command execution', {
      command,
      workspacePath: this.workspacePath,
      timeout,
      envVarCount: Object.keys(this.repositoryConfig.codeExecution.envVars || {}).length,
    })

    const startTime = Date.now()

    try {
      // Create and execute a new shell command
      const commandId = this.generateCommandId()
      const shellCommand = new ShellCommand()
      this.shellCommands.set(commandId, shellCommand)

      const result = await shellCommand.executeWithResult(command, {
        workingDirectory: this.workspacePath,
        env: {
          ...this.repositoryConfig.codeExecution.envVars,
          ...additionalEnv,
        },
        timeout,
      })

      // Clean up completed command
      this.shellCommands.delete(commandId)

      this.logger.info('Command execution completed', {
        command,
        exitCode: result.exitCode,
        durationMs: result.durationMs,
        outputLength: result.output.length,
      })

      return result
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.logger.error('Command execution failed', error as Error, {
        command,
        workspacePath: this.workspacePath,
        errorMessage,
      })

      return {
        command,
        exitCode: 1,
        output: errorMessage,
        durationMs: Date.now() - startTime,
        success: false,
        error: errorMessage,
      }
    }
  }

  private generateCommandId(): string {
    this.commandCounter++
    return `${this.taskId}-cmd-${this.commandCounter}-${Date.now()}`
  }

  async *runCommandWithStream(
    command: string,
    timeout = 30000,
  ): AsyncGenerator<ShellCommandLogEvent> {
    this.logger.info('Starting streaming command execution', {
      command,
      workspacePath: this.workspacePath,
      timeout,
    })

    try {
      // Create and execute a new shell command in background
      const commandId = this.generateCommandId()
      const shellCommand = new ShellCommand()
      this.shellCommands.set(commandId, shellCommand)

      await shellCommand.execute(command, {
        workingDirectory: this.workspacePath,
        env: this.repositoryConfig.codeExecution.envVars,
        timeout,
        background: true, // Run in background for streaming
      })

      // Stream logs from the shell command (including system messages for start/complete/error)
      for await (const logEvent of shellCommand.streamLogs()) {
        yield logEvent
      }

      // Clean up completed command
      this.shellCommands.delete(commandId)

      const commandInfo = shellCommand.getInfo()
      this.logger.info('Streaming command execution completed', {
        command,
        exitCode: commandInfo?.exitCode || 0,
        duration: commandInfo?.duration,
      })
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)

      this.logger.error('Streaming command execution failed', error as Error, {
        command,
        errorMessage,
      })

      // Emit error as system log
      yield {
        type: 'system',
        timestamp: new Date().toISOString(),
        text: `Error: ${errorMessage}\n`,
      }
    }
  }

  async startServer(): Promise<ServerStatus> {
    // Clear logs from previous server runs
    await clearTaskLog(this.taskId)
    this.logger.info('Cleared task logs for new server session')

    // Run setup commands if not already executed (logs will be included)
    if (!this.setupCommandsExecuted) {
      await this.runSetupCommands()
    }

    // Get server commands
    const serverStartCommands = this.repositoryConfig.previewConfig?.serverStartCommands

    let commands: string[]
    if (serverStartCommands && serverStartCommands.length > 0) {
      // Filter out empty commands
      commands = serverStartCommands.filter((cmd) => cmd.trim() !== '')
    } else {
      throw new Error('No server commands configured')
    }

    // Ensure we have at least one valid command after filtering
    if (commands.length === 0) {
      throw new Error('No valid server commands configured (all commands are empty)')
    }

    // Check if server is already running
    if (this.previewServer) {
      const status = this.previewServer.getStatus()
      if (status.status === 'started') {
        this.logger.info('Server already running', { port: status.url })
        return status
      }
    }

    this.logger.info('Starting preview server', {
      commands: commands,
      workspacePath: this.workspacePath,
    })

    try {
      // Create server handler if it doesn't exist
      if (!this.previewServer) {
        this.previewServer = new PreviewServer(
          this.taskId,
          this.workspacePath,
          this.eventService,
          this.logFile,
        )
      }

      // Start the server with latest configuration
      const result = await this.previewServer.start(
        commands,
        this.repositoryConfig.codeExecution.envVars || {},
        this.repositoryConfig,
      )

      if (result.status === 'starting') {
        this.logger.info('Preview server is starting', {
          pid: result.pid,
          logFile: result.log_file,
        })

        return {
          status: 'starting',
          pid: result.pid,
          uptimeSeconds: 0,
        }
      }

      if (result.status === 'failed') {
        throw new Error(result.error || 'Failed to start preview server')
      }

      // This shouldn't happen, but handle it
      throw new Error('Unexpected server start result')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.logger.error('Failed to start preview server', {
        error: errorMessage,
        command: commands.join(' && '),
        workspacePath: this.workspacePath,
      })

      // Return failed state
      return {
        status: 'failed',
        uptimeSeconds: 0,
        error: errorMessage,
        logs: 'Failed to start server',
      }
    }
  }

  async stopServer(): Promise<boolean> {
    if (!this.previewServer) {
      this.logger.debug('No server handler found')
      return false
    }

    try {
      // Stop the server (it handles proxy and metadata cleanup internally)
      const stopped = await this.previewServer.stop()
      if (stopped) {
        // Don't set previewServer to undefined - we want to reuse it for restart
        // The PreviewServer.stop() method now preserves the port allocation
        this.logger.info('Preview server stopped successfully')
      }
      return stopped
    } catch (error) {
      this.logger.error('Error stopping preview server', error as Error)
      // Force cleanup
      this.previewServer = undefined
      return false
    }
  }

  async getServerStatus(): Promise<ServerStatus> {
    // Check running servers
    if (this.previewServer) {
      return this.previewServer.getStatus()
    }

    return {
      status: 'stopped',
      uptimeSeconds: 0,
    }
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (this.previewServer) {
      return await this.previewServer.getLogs(lines)
    }
    return 'No server logs available'
  }

  async *streamServerLogs(): AsyncGenerator<{ type: string; data: any }> {
    if (this.previewServer) {
      // Stream logs from active preview server
      for await (const logEvent of this.previewServer.streamLogs()) {
        yield {
          type: logEvent.type,
          data: logEvent.data,
        }
      }
    } else {
      // No preview server running
      yield {
        type: 'error',
        data: {
          error: 'No server logs available',
          timestamp: new Date().toISOString(),
        },
      }
      yield {
        type: 'complete',
        data: {
          timestamp: new Date().toISOString(),
        },
      }
    }
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

  /**
   * Construct the MCP server URL for local environment
   */
  private getMcpServerUrl(taskId: string): string {
    // For local environment, use TEST_SERVER_URL if in test, or localhost with current port
    const serverUrl =
      process.env['TEST_SERVER_URL'] || `http://localhost:${process.env['PORT'] || 8000}`
    return `${serverUrl}/api/v1/tasks/${taskId}/mcp`
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
      agentMode?: 'edit' | 'plan'
      hasPreviewServer?: boolean
    },
  ): AsyncGenerator<any> {
    this.logger.info('Running agent', { agentType, taskId: this.taskId })

    // Create agent options
    const agentOptions = options?.model ? { model: options.model } : {}

    // Create agent instance
    const agent = await this.createAgent(agentType, agentOptions)
    this.currentAgent = agent

    try {
      // Construct MCP server URL
      const mcpServerUrl = this.getMcpServerUrl(this.taskId)

      // Prepare run options
      const runOptions = {
        workspaceDir: options?.workspaceDir || this.workspacePath,
        iterationNumber: options?.iterationNumber,
        resume: options?.resume,
        model: options?.model,
        agentMode: options?.agentMode,
        mcpServerUrl,
        hasPreviewServer: options?.hasPreviewServer,
      }

      // Run the agent and yield all events
      for await (const event of agent.run(message, runOptions)) {
        yield event
      }
    } finally {
      // Clean up agent
      await this.cleanupAgent()
    }
  }

  async stopAgent(): Promise<boolean> {
    if (!this.currentAgent) {
      return false
    }

    this.logger.info('Stopping running agent')

    try {
      await this.currentAgent.stop()
      await this.cleanupAgent()
      return true
    } catch (error) {
      this.logger.error('Error stopping agent', error as Error)
      await this.cleanupAgent()
      return false
    }
  }

  /**
   * Clean up agent (simplified)
   */
  private async cleanupAgent(): Promise<void> {
    // Clear the current agent
    this.currentAgent = undefined
  }

  async teardown(force = false): Promise<void> {
    this.logger.info('Tearing down LocalEnvironment', { force })

    try {
      // Stop agent
      await this.stopAgent()

      // Stop and dispose server (releases port)
      if (this.previewServer) {
        await this.previewServer.dispose()
        this.previewServer = undefined
      }

      this.logger.info('LocalEnvironment teardown completed')
    } catch (error) {
      this.logger.error('Error during teardown', error as Error)
      if (!force) {
        throw error
      }
    }
  }

  // ===============================
  // Build and Deploy Methods
  // ===============================

  async buildProject(): Promise<{
    success: boolean
    outputDirectory: string
    logs: string
    duration: number
  }> {
    const buildCommand = this.repositoryConfig.previewConfig?.buildCommand
    if (!buildCommand) {
      throw new Error('No build command configured in repository preview settings')
    }

    this.logger.info('Starting project build', {
      buildCommand,
      workspacePath: this.workspacePath,
    })

    const startTime = Date.now()
    const result = await this.runCommand(buildCommand, 300000) // 5 minute timeout

    const outputDir = this.repositoryConfig.previewConfig?.buildOutputDirectory || 'dist'
    const outputPath = path.join(this.workspacePath, outputDir)

    // Verify output directory exists
    const outputExists = await pathExists(outputPath)
    const duration = Date.now() - startTime

    this.logger.info('Build completed', {
      success: result.exitCode === 0 && outputExists,
      exitCode: result.exitCode,
      outputExists,
      duration,
      outputPath,
    })

    return {
      success: result.exitCode === 0 && outputExists,
      outputDirectory: outputPath,
      logs: result.output,
      duration,
    }
  }

  async deployToNetlify(
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
  }> {
    const outputDir = this.repositoryConfig.previewConfig?.buildOutputDirectory || 'dist'
    const outputPath = path.join(this.workspacePath, outputDir)

    // Generate site name if not provided
    const finalSiteName = siteName || `moatless-${this.taskId.substring(0, 8)}`

    this.logger.info('Deploying to Netlify via API', {
      siteName: finalSiteName,
      outputPath,
      existingSiteId,
    })

    // Use the NetlifyService (will be injected via dependency injection)
    // For now, create a temporary instance - this will be replaced when we wire DI
    const { createNetlifyService } = await import('../services/netlify.service.js')
    const netlifyService = createNetlifyService()

    return await netlifyService.deployToNetlify(
      outputPath,
      finalSiteName,
      authToken,
      existingSiteId,
    )
  }
}
