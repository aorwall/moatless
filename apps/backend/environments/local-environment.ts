import { exec, spawn } from 'node:child_process'
import path from 'node:path'
import { promisify } from 'node:util'
import type {
  CommandExecutionEvent,
  CommandResult,
  RepositoryConfig,
  ServerStatus,
} from '@moatless/types'
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

      // Run setup commands synchronously
      const setupCommands = this.repositoryConfig.codeExecution.setupCommands || []
      this.logger.info(`Found ${setupCommands.length} setup commands to run`, { setupCommands })

      for (const command of setupCommands) {
        this.logger.info(`Running setup command: ${command}`)
        const commandStartTime = Date.now()
        const result = await this.runCommand(command, 300000) // 5 minutes timeout
        const commandDuration = Date.now() - commandStartTime

        this.logger.info('Setup command completed', {
          command,
          exitCode: result.exitCode,
          durationMs: commandDuration,
          outputLength: result.output.length,
          output: result.output.slice(0, 500) + (result.output.length > 500 ? '...' : ''), // First 500 chars
        })

        if (result.exitCode !== 0) {
          throw new Error(
            `Setup command failed: ${command}. Exit code: ${result.exitCode}. Output: ${result.output}`,
          )
        }
      }

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
      const oldConfig = this.repositoryConfig
      ;(this as any).repositoryConfig = latestConfig // Cast to bypass readonly

      // Step 3: Re-run setup commands
      const setupCommands = latestConfig.codeExecution.setupCommands || []
      this.logger.info(`Running ${setupCommands.length} setup commands with new configuration`, {
        setupCommands,
      })

      for (const command of setupCommands) {
        this.logger.info(`Running setup command: ${command}`)
        const commandStartTime = Date.now()
        const result = await this.runCommand(command, 300000) // 5 minutes timeout
        const commandDuration = Date.now() - commandStartTime

        this.logger.info('Setup command completed', {
          command,
          exitCode: result.exitCode,
          durationMs: commandDuration,
          outputLength: result.output.length,
          output: result.output.slice(0, 500) + (result.output.length > 500 ? '...' : ''),
        })

        if (result.exitCode !== 0) {
          this.logger.error('Setup command failed during refresh', {
            command,
            exitCode: result.exitCode,
            output: result.output,
          })
          // Continue with other commands instead of throwing
          // This ensures partial refresh can succeed
        }
      }

      const totalDuration = Date.now() - startTime
      this.logger.info('Configuration refresh completed successfully', {
        durationMs: totalDuration,
        commandsRun: setupCommands.length,
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
  ): Promise<CommandResult> {
    this.logger.info('Starting command execution', {
      command,
      workspacePath: this.workspacePath,
      timeout,
      envVarCount: Object.keys(this.repositoryConfig.codeExecution.envVars || {}).length,
    })

    try {
      // Create a shell command that sources the profile and then executes the command
      // This ensures nvm and other shell functions are available
      const shellCommand =
        process.platform === 'win32'
          ? command // Windows doesn't need special handling for nvm
          : `source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null || true; ${command}`

      const execOptions = {
        cwd: this.workspacePath,
        timeout,
        maxBuffer: 1024 * 1024, // 1MB buffer
        env: {
          ...process.env,
          ...this.repositoryConfig.codeExecution.envVars,
          ...additionalEnv,
        },
        shell: process.platform === 'win32' ? 'cmd.exe' : process.env.SHELL || '/bin/zsh',
      }

      this.logger.debug('Exec options', {
        cwd: execOptions.cwd,
        timeout: execOptions.timeout,
        shell: execOptions.shell,
        envKeys: Object.keys(execOptions.env).slice(0, 10), // First 10 env vars
        hasNpmPath: execOptions.env.PATH?.includes('npm') || false,
      })

      const { stdout, stderr } = await execAsync(shellCommand, execOptions)

      // Log stderr for debugging if present
      if (stderr) {
        this.logger.warn(`Command stderr: ${stderr}`)
      }

      this.logger.info('Command execution successful', {
        command,
        stdoutLength: stdout.length,
        stderrLength: stderr.length,
      })

      return {
        exitCode: 0,
        output: stdout.trim() + (stderr ? `\n${stderr.trim()}` : ''),
        durationMs: 0,
        command,
      }
    } catch (error) {
      const execError = error as Error & { code?: number; stdout?: string; stderr?: string }
      this.logger.error('Command execution failed', execError, {
        command,
        workspacePath: this.workspacePath,
        errorCode: execError.code,
        stdout: execError.stdout,
        stderr: execError.stderr,
      })

      return {
        exitCode: execError.code || 1,
        output: execError.stdout?.trim() || execError.stderr?.trim() || execError.message || '',
        durationMs: 0,
        command,
      }
    }
  }

  async *runCommandWithStream(
    command: string,
    timeout = 30000,
  ): AsyncGenerator<CommandExecutionEvent> {
    this.logger.info('Starting streaming command execution', {
      command,
      workspacePath: this.workspacePath,
      timeout,
    })

    const startTime = Date.now()

    // Emit start event
    yield {
      type: 'start',
      data: {
        timestamp: new Date().toISOString(),
        command,
      },
    }

    // Create the shell command
    const shellCommand =
      process.platform === 'win32'
        ? command
        : `source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null || true; ${command}`

    const shell = process.platform === 'win32' ? 'cmd.exe' : process.env.SHELL || '/bin/zsh'

    // Use spawn for streaming output
    const child = spawn(shell, ['-c', shellCommand], {
      cwd: this.workspacePath,
      env: { ...process.env, ...this.repositoryConfig.codeExecution.envVars },
      timeout,
    })

    let outputBuffer = ''
    let errorBuffer = ''
    let timedOut = false

    // Set up timeout
    const timeoutHandle = setTimeout(() => {
      timedOut = true
      child.kill('SIGTERM')
    }, timeout)

    // Set up arrays to collect stderr chunks
    const stderrChunks: Array<{ text: string; timestamp: string }> = []

    // Collect stderr
    if (child.stderr) {
      child.stderr.on('data', (chunk) => {
        const text = chunk.toString()
        errorBuffer += text
        stderrChunks.push({
          text,
          timestamp: new Date().toISOString(),
        })
      })
    }

    // Stream stdout and periodically check for stderr
    if (child.stdout) {
      for await (const chunk of child.stdout) {
        const text = chunk.toString()
        outputBuffer += text

        // Yield stdout
        yield {
          type: 'output',
          data: {
            timestamp: new Date().toISOString(),
            text,
            outputType: 'stdout',
          },
        }

        // Yield any accumulated stderr
        while (stderrChunks.length > 0) {
          const stderrChunk = stderrChunks.shift()!
          yield {
            type: 'output',
            data: {
              timestamp: stderrChunk.timestamp,
              text: stderrChunk.text,
              outputType: 'stderr',
            },
          }
        }
      }
    }

    // Wait for process to exit
    const exitCode = await new Promise<number>((resolve) => {
      child.on('exit', (code) => {
        clearTimeout(timeoutHandle)
        resolve(code || 0)
      })
      child.on('error', (error) => {
        clearTimeout(timeoutHandle)
        this.logger.error('Command execution error', error)
        resolve(1)
      })
    })

    // Yield any remaining stderr
    while (stderrChunks.length > 0) {
      const stderrChunk = stderrChunks.shift()!
      yield {
        type: 'output',
        data: {
          timestamp: stderrChunk.timestamp,
          text: stderrChunk.text,
          outputType: 'stderr',
        },
      }
    }

    const duration = Date.now() - startTime

    // Emit complete event
    yield {
      type: 'complete',
      data: {
        timestamp: new Date().toISOString(),
        exitCode: timedOut ? 124 : exitCode, // 124 is standard timeout exit code
        duration,
      },
    }

    this.logger.info('Streaming command execution completed', {
      command,
      exitCode,
      duration,
      timedOut,
    })
  }

  async startServer(): Promise<ServerStatus> {
    // Get server commands with backward compatibility
    const serverStartCommands = this.repositoryConfig.previewConfig?.serverStartCommands
    const serverStartCommand = this.repositoryConfig.previewConfig?.serverStartCommand

    let commands: string[]
    if (serverStartCommands && serverStartCommands.length > 0) {
      // Use new array format, filter out empty commands
      commands = serverStartCommands.filter((cmd) => cmd.trim() !== '')
    } else if (serverStartCommand) {
      // Fall back to legacy single command
      commands = [serverStartCommand]
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
        this.previewServer = new PreviewServer(this.taskId, this.workspacePath, this.eventService)
      }

      // Start the server with configuration
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
        this.previewServer = undefined
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
    const serverUrl = process.env.TEST_SERVER_URL || `http://localhost:${process.env.PORT || 8000}`
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
    const outputExists = await this.pathExists(outputPath)
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
