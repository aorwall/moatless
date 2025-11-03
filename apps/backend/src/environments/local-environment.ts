import { randomUUID } from 'node:crypto'
import path from 'node:path'
import { ShellCommand, type ShellCommandOptions } from '@moatless/shell-utils'
import type {
  RepositoryConfig,
  ServerStatus,
  ShellCommandLogEvent,
  ShellCommandResult,
} from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../features/agents/index'
import { ClaudeCodeAgent } from '../features/agents/index'
import type { EventService } from '../features/events/index'
import { PreviewServer } from '../features/preview/runtime/preview-server.js'
import type {
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerStatus,
  ServerStatusChangeInfo,
} from '../features/preview/types/index.js'
import { pathExists } from '../utils/filesystem'
import { createTaskLogger } from '../utils/logger'
import { clearTaskLog } from '../utils/task-log'
import { BaseEnvironment } from './base-environment'

/**
 * Convert PreviewServerStatus → ServerStatus
 * Maps feature module preview server status to moatless-specific status
 */
function toServerStatus(status: PreviewServerStatus): ServerStatus {
  return {
    status: status.status,
    pid: status.pid,
    url: status.proxyUrl || status.url, // Use proxy URL if available, otherwise direct server URL
    uptimeSeconds: status.uptimeSeconds || 0,
    currentPath: status.currentPath,
    error: status.error,
    logs: status.logs,
  }
}

export class LocalEnvironment extends BaseEnvironment {
  private setupInProgress = false
  private setupComplete = false
  private setupCommandsExecuted = false
  private workspacePath = ''
  private _createdAt = new Date()
  private _error: string | undefined
  private logger = createTaskLogger(this.taskId)
  private previewServer: PreviewServer | undefined
  private previewLifecycleSink: PreviewServerLifecycleSink | undefined
  private shellCommands: Map<string, ShellCommand> = new Map()
  private commandCounter = 0
  private logFile: string | undefined

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath?: string,
    eventService?: EventService,
    logFile?: string,
    previewLifecycleSink?: PreviewServerLifecycleSink,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.workspacePath = workspacePath || ''
    this.logFile = logFile
    this.logger = createTaskLogger(taskId)
    this.previewLifecycleSink = previewLifecycleSink

    this.logger.debug('LocalEnvironment constructor', {
      repositoryId: repositoryConfig.id,
      repositoryPath: repositoryConfig.path,
      workspacePath: this.workspacePath,
    })

    // DO NOT try to recover existing servers - it's unsafe and creates fake process objects
  }

  getType(): 'local' {
    return 'local'
  }

  getStatus(): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) {
      return 'error'
    }
    if (this.setupInProgress) {
      return 'initializing'
    }
    if (this.setupComplete) {
      return 'ready'
    }
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
    if (!this.eventService) {
      return
    }

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

      this.logger.debug('Server event emitted during setup', {
        status,
        taskId: this.taskId,
      })
    } catch (eventError) {
      this.logger.error('Failed to emit server event during setup', eventError as Error)
    }
  }

  /**
   * Create a lifecycle sink that wraps the original sink and adds event conversion
   * This converts generic preview server status changes to moatless events with taskId
   */
  private createLifecycleSinkWithEventConversion(): PreviewServerLifecycleSink {
    const lifecycleSink: PreviewServerLifecycleSink = {
      register: async (serverId: string, snapshot) => {
        if (this.previewLifecycleSink?.register) {
          await this.previewLifecycleSink.register(serverId, snapshot)
        }
      },
      update: async (serverId: string, changes) => {
        if (this.previewLifecycleSink?.update) {
          await this.previewLifecycleSink.update(serverId, changes)
        }
      },
      remove: async (serverId: string) => {
        if (this.previewLifecycleSink?.remove) {
          await this.previewLifecycleSink.remove(serverId)
        }
      },
      onStatusChange: async (serverId: string, statusInfo: ServerStatusChangeInfo) => {
        // First, delegate to the original lifecycle sink if it has onStatusChange
        if (this.previewLifecycleSink?.onStatusChange) {
          await this.previewLifecycleSink.onStatusChange(serverId, statusInfo)
        }

        // Then, convert to moatless event with taskId and emit
        if (this.eventService) {
          try {
            await this.eventService.emitEvent({
              eventId: randomUUID(),
              type: 'server' as const,
              subType: statusInfo.status,
              taskId: this.taskId,
              timestamp: new Date(),
              isError: statusInfo.status === 'failed',
              payload: {
                status: statusInfo.status,
                port: statusInfo.port,
                pid: statusInfo.pid,
                url: statusInfo.url,
                error: statusInfo.error,
                uptimeSeconds: statusInfo.uptimeSeconds,
              },
            })

            this.logger.debug('Server status change converted to moatless event', {
              status: statusInfo.status,
              taskId: this.taskId,
              serverId,
            })
          } catch (eventError) {
            this.logger.error('Failed to emit server status change event', eventError as Error, {
              status: statusInfo.status,
              taskId: this.taskId,
            })
          }
        }
      },
    }

    return lifecycleSink
  }

  async runSetupCommands(): Promise<void> {
    const setupCommands = this.repositoryConfig.codeExecution.setupCommands || []

    if (this.shouldSkipSetupCommands(setupCommands)) {
      return
    }

    this.logger.info(`Running ${setupCommands.length} setup commands`, { setupCommands })

    const hasPreviewConfig = this.hasPreviewConfiguration()
    await this.emitInstallingStatusIfNeeded(hasPreviewConfig)

    try {
      await this.executeAllSetupCommands(setupCommands, hasPreviewConfig)
      this.setupCommandsExecuted = true
      this.logger.info('All setup commands completed successfully')
    } catch (error) {
      await this.handleSetupCommandsError(error, hasPreviewConfig)
      throw error
    }
  }

  private shouldSkipSetupCommands(setupCommands: string[]): boolean {
    if (setupCommands.length === 0) {
      this.logger.info('No setup commands configured, marking as executed')
      this.setupCommandsExecuted = true
      return true
    }

    if (this.setupCommandsExecuted) {
      this.logger.info('Setup commands already executed, skipping')
      return true
    }

    return false
  }

  private hasPreviewConfiguration(): boolean {
    return (this.repositoryConfig.previewConfig?.serverStartCommands?.length ?? 0) > 0
  }

  private async emitInstallingStatusIfNeeded(hasPreviewConfig: boolean): Promise<void> {
    if (hasPreviewConfig && this.eventService) {
      await this.emitServerEvent('installing')
    }
  }

  private async executeAllSetupCommands(
    setupCommands: string[],
    hasPreviewConfig: boolean,
  ): Promise<void> {
    for (const command of setupCommands) {
      await this.executeSingleSetupCommand(command, hasPreviewConfig)
    }
  }

  private async executeSingleSetupCommand(
    command: string,
    hasPreviewConfig: boolean,
  ): Promise<void> {
    this.logger.info(`Running setup command: ${command}`)
    const commandStartTime = Date.now()

    const { commandId, shellCommand } = this.createShellCommand()
    const result = await this.runCommandWithTimeout(shellCommand, command)
    this.shellCommands.delete(commandId)

    const commandDuration = Date.now() - commandStartTime
    this.logCommandCompletion(command, result, commandDuration)

    if (result.exitCode !== 0) {
      await this.handleCommandFailure(command, result, hasPreviewConfig)
    }
  }

  private createShellCommand(): { commandId: string; shellCommand: ShellCommand } {
    const commandId = this.generateCommandId()
    const shellCommand = new ShellCommand()
    this.shellCommands.set(commandId, shellCommand)
    return { commandId, shellCommand }
  }

  private async runCommandWithTimeout(
    shellCommand: ShellCommand,
    command: string,
  ): Promise<{ exitCode: number; output: string }> {
    const options: ShellCommandOptions = {
      workingDirectory: this.workspacePath,
      timeout: 300000, // 5 minutes timeout
    }

    if (this.repositoryConfig.codeExecution.envVars !== undefined) {
      options.env = this.repositoryConfig.codeExecution.envVars
    }

    if (this.logFile !== undefined) {
      options.logFile = this.logFile
    }

    return await shellCommand.executeWithResult(command, options)
  }

  private logCommandCompletion(
    command: string,
    result: { exitCode: number; output: string },
    durationMs: number,
  ): void {
    const outputPreview = result.output.slice(0, 500) + (result.output.length > 500 ? '...' : '')

    this.logger.info('Setup command completed', {
      command,
      exitCode: result.exitCode,
      durationMs,
      outputLength: result.output.length,
      output: outputPreview,
    })
  }

  private async handleCommandFailure(
    command: string,
    result: { exitCode: number; output: string },
    hasPreviewConfig: boolean,
  ): Promise<void> {
    if (hasPreviewConfig && this.eventService) {
      await this.emitServerEvent('failed', `Setup command failed: ${command}`)
    }

    throw new Error(
      `Setup command failed: ${command}. Exit code: ${result.exitCode}. Output: ${result.output}`,
    )
  }

  private async handleSetupCommandsError(error: unknown, hasPreviewConfig: boolean): Promise<void> {
    if (hasPreviewConfig && this.eventService) {
      const errorMsg = error instanceof Error ? error.message : String(error)
      await this.emitServerEvent('failed', errorMsg)
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
      // biome-ignore lint/suspicious/noExplicitAny: intentional cast to bypass readonly property for config refresh
      ;(this as any).repositoryConfig = latestConfig

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

      const options: ShellCommandOptions = {
        workingDirectory: this.workspacePath,
        timeout,
        background: true, // Run in background for streaming
      }

      if (this.repositoryConfig.codeExecution.envVars !== undefined) {
        options.env = this.repositoryConfig.codeExecution.envVars
      }

      await shellCommand.execute(command, options)

      // Stream logs from the shell command (including system messages for start/complete/error)
      for await (const logEvent of shellCommand.streamLogs()) {
        yield logEvent
      }

      // Clean up completed command
      this.shellCommands.delete(commandId)

      this.logger.info('Streaming command execution completed', {
        command,
        exitCode: shellCommand.getExitCode() || 0,
        duration: shellCommand.getDuration(),
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

    // Check if server is already running
    if (this.previewServer) {
      const status = this.previewServer.getStatus()
      if (status.status === 'started') {
        this.logger.info('Server already running', { port: status.url })
        return toServerStatus(status)
      }
    }

    this.logger.info('Starting preview server', {
      workspacePath: this.workspacePath,
    })

    try {
      // Create preview config directly
      const previewConfig: PreviewServerConfig = {
        serverId: this.taskId,
        workspacePath: this.workspacePath,
        setupCommands: this.repositoryConfig.codeExecution.setupCommands || [],
        serverCommands: this.repositoryConfig.previewConfig?.serverStartCommands || [],
        envVars: this.repositoryConfig.codeExecution.envVars,
        startupTimeoutSeconds: this.repositoryConfig.previewConfig?.startupTimeoutSeconds,
        enableInspector: this.repositoryConfig.previewConfig?.enableInspector,
        logFile: this.logFile,
      }

      // Create server handler if it doesn't exist
      if (!this.previewServer) {
        this.previewServer = new PreviewServer(
          previewConfig,
          this.createLifecycleSinkWithEventConversion(),
        )
      }

      // Start the server
      const result = await this.previewServer.start()

      if (result.status === 'starting') {
        this.logger.info('Preview server is starting', {
          pid: result.pid,
        })

        return {
          status: 'starting',
          pid: result.pid,
          uptimeSeconds: 0,
        }
      }

      if (result.status === 'started') {
        this.logger.info('Preview server started successfully', {
          pid: result.pid,
          url: result.proxyUrl || result.url,
        })

        return {
          status: 'started',
          url: result.proxyUrl || result.url,
          pid: result.pid,
          uptimeSeconds: result.uptimeSeconds || 0,
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

  getServerStatus(): Promise<ServerStatus> {
    // Check running servers
    if (this.previewServer) {
      const status = this.previewServer.getStatus()
      return Promise.resolve(toServerStatus(status))
    }

    return Promise.resolve({
      status: 'stopped',
      uptimeSeconds: 0,
    })
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (this.previewServer) {
      return await this.previewServer.getLogs(lines)
    }

    // Try reading from log file directly
    if (this.logFile) {
      const { promises: fs } = await import('node:fs')
      try {
        const content = await fs.readFile(this.logFile, 'utf-8')
        const logLines = content.split('\n')
        return logLines.slice(-lines).join('\n')
      } catch {
        return 'No server logs available'
      }
    }

    return 'No server logs available'
  }

  async *streamServerLogs(): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    if (this.previewServer) {
      // Stream logs from active preview server
      for await (const logEvent of this.previewServer.streamLogs()) {
        yield {
          type: logEvent.type,
          data: logEvent.data,
        }
      }
    } else if (this.logFile) {
      // No preview server, but we have a log file - read directly
      yield* this.streamLogsFromFile()
    } else {
      // No server and no log file
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

  /**
   * Stream logs directly from log file when PreviewServer is not available
   */
  private async *streamLogsFromFile(): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    const { promises: fs } = await import('node:fs')

    // Guard check - this should never happen since this method is only called when logFile exists
    if (!this.logFile) {
      yield {
        type: 'error',
        data: {
          error: 'No log file available',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    try {
      // Read existing log content
      const content = await fs.readFile(this.logFile, 'utf-8')
      if (content) {
        yield {
          type: 'chunk',
          data: {
            text: content,
            timestamp: new Date().toISOString(),
          },
        }
      }

      // Signal completion
      yield {
        type: 'complete',
        data: {
          timestamp: new Date().toISOString(),
        },
      }
    } catch (error) {
      yield {
        type: 'error',
        data: {
          error: error instanceof Error ? error.message : String(error),
          timestamp: new Date().toISOString(),
        },
      }
    }
  }

  getPreviewPort(): number | undefined {
    return this.previewServer?.getPort()
  }

  proxyPreviewRequest(request: {
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
      return Promise.reject(new Error('Preview server not running'))
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

  createAgent(agentType: string, options?: AgentOptions): Promise<BaseAgent> {
    this.logger.info('Creating agent', { agentType, taskId: this.taskId })

    if (agentType === 'claude-code') {
      return Promise.resolve(new ClaudeCodeAgent(this.taskId, options))
    }
    return Promise.reject(new Error(`Unknown agent type: ${agentType}`))
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
      appendSystemPrompt?: string
    },
    // biome-ignore lint/suspicious/noExplicitAny: Event type yields from agent, complex union type
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
        ...(options?.iterationNumber !== undefined && {
          iterationNumber: options.iterationNumber,
        }),
        ...(options?.resume !== undefined && { resume: options.resume }),
        ...(options?.model !== undefined && { model: options.model }),
        ...(options?.agentMode !== undefined && {
          agentMode: options.agentMode,
        }),
        mcpServerUrl,
        ...(options?.hasPreviewServer !== undefined && {
          hasPreviewServer: options.hasPreviewServer,
        }),
        ...(options?.appendSystemPrompt !== undefined && {
          appendSystemPrompt: options.appendSystemPrompt,
        }),
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
  private cleanupAgent(): Promise<void> {
    // Clear the current agent
    this.currentAgent = undefined
    return Promise.resolve()
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

    // Use the NetlifyService via inversify
    const { appContainer } = await import('../container/container.js')
    const { NetlifyService } = await import('../services/netlify.service.js')
    const netlifyService = appContainer.get(NetlifyService)

    return await netlifyService.deployToNetlify(
      outputPath,
      finalSiteName,
      authToken,
      existingSiteId,
    )
  }
}
