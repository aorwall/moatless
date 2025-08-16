import { exec, execSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import * as net from 'node:net'
import path from 'node:path'
import { promisify } from 'node:util'
import type {
  CommandResult,
  PreviewEnvironmentMetadata,
  ProcessValidation,
  RepositoryConfig,
  ServerStatus,
} from '@moatless/types'
import type { Logger } from 'pino'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import { ClaudeCodeAgent } from '../agents/claude-code/claude-code-agent.js'
import type { EventService } from '../services/event.service.js'
import { createPreviewProxy, type PreviewProxyService } from '../services/preview-proxy.service.js'
import { PreviewServer } from '../services/preview-server.js'
import { getDataDir } from '../utils/data-dir.js'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
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
  private previewProxy?: PreviewProxyService
  private readonly previewEnvironmentsPath: string

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath?: string,
    eventService?: EventService,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.workspacePath = workspacePath || ''
    this.logger = createTaskLogger(taskId)

    // Initialize preview environments storage path
    this.previewEnvironmentsPath = path.join(getDataDir(), 'preview-environments')
    this.initializeStorage()

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

  async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
    this.logger.info('Starting command execution', {
      command,
      workspacePath: this.workspacePath,
      timeout,
      envVarCount: Object.keys(this.repositoryConfig.codeExecution.envVars || {}).length,
    })

    try {
      const execOptions = {
        cwd: this.workspacePath,
        timeout,
        maxBuffer: 1024 * 1024, // 1MB buffer
        env: { ...process.env, ...this.repositoryConfig.codeExecution.envVars },
      }

      this.logger.debug('Exec options', {
        cwd: execOptions.cwd,
        timeout: execOptions.timeout,
        envKeys: Object.keys(execOptions.env).slice(0, 10), // First 10 env vars
        hasNpmPath: execOptions.env.PATH?.includes('npm') || false,
      })

      const { stdout, stderr } = await execAsync(command, execOptions)

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

  async startServer(): Promise<ServerStatus> {
    const serverCommand = this.repositoryConfig.previewConfig?.serverStartCommand
    if (!serverCommand) {
      throw new Error('No server command configured')
    }

    // Check if server is already running
    const existingServer = this.previewServer
    if (existingServer) {
      const status = existingServer.getStatus()
      if (status.running) {
        this.logger.info('Server already running', { port: status.url })
        return status
      }
    }

    this.logger.info('Starting preview server', {
      command: serverCommand,
      framework: this.repositoryConfig.previewConfig?.framework,
      workspacePath: this.workspacePath,
    })

    try {
      // Create server handler
      this.previewServer = new PreviewServer(
        this.taskId,
        this.workspacePath,
        serverCommand,
        this.repositoryConfig.codeExecution.envVars || {},
      )

      // Start the server first
      const result = await this.previewServer.start()

      // Try to create and start proxy server for SPA support
      let proxyUrl: string | undefined
      let proxyPort: number | undefined

      try {
        proxyPort = await findAvailablePort('general')
        const targetUrl = `http://localhost:${result.port}`
        const scriptContent = `
          console.log('🚀 Moatless Preview Script Injected for task: ${this.taskId}');
          
          // Send a message to parent to confirm script is loaded
          window.addEventListener('load', () => {
            if (window.parent && window.parent !== window) {
              window.parent.postMessage({
                type: 'INSPECTOR_SCRIPT_LOADED',
                payload: { version: '1.0.0', taskId: '${this.taskId}' },
                source: 'preview-iframe'
              }, '*');
            }
          });
        `

        this.previewProxy = createPreviewProxy(
          targetUrl,
          scriptContent,
          this.logger as Logger,
          proxyPort,
        )
        await this.previewProxy.start()

        proxyUrl = `http://localhost:${proxyPort}`
        this.logger.info('Preview proxy started successfully', { proxyPort, proxyUrl })
      } catch (error) {
        this.logger.warn('Failed to start preview proxy, falling back to path-based proxy', {
          error: error instanceof Error ? error.message : String(error),
          proxyPort,
        })
        // Reset proxy-related variables since proxy failed
        this.previewProxy = undefined
        proxyPort = undefined
        proxyUrl = undefined
      }

      // Save metadata to disk (successful startup, clear any previous error state)
      const metadata: PreviewEnvironmentMetadata = {
        taskId: this.taskId,
        pid: result.pid,
        port: result.port,
        startTime: new Date(),
        command: serverCommand,
        workspacePath: this.workspacePath,
        logFile: result.log_file,
        processStartTime: Date.now(),
        envVars: this.repositoryConfig.codeExecution.envVars,
        failed: false, // Explicitly mark as not failed
        error: undefined, // Clear any previous error
        logs: undefined, // Clear any previous error logs
        proxyPort,
        proxyUrl,
      }

      await this.savePreviewEnvironmentMetadata(this.taskId, metadata)

      this.logger.info('Preview server started successfully', {
        port: result.port,
        proxyPort: proxyPort || 'not started',
        proxyUrl: proxyUrl || 'not available',
        pid: result.pid,
      })

      return {
        running: true,
        configured: true,
        url: `http://localhost:${result.port}`,
        proxyUrl,
        pid: result.pid,
        uptimeSeconds: 0,
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.logger.error('Failed to start preview server', {
        error: errorMessage,
        command: serverCommand,
        workspacePath: this.workspacePath,
      })

      // Try to get server logs for better error context
      let logs = ''
      try {
        if (this.previewServer) {
          logs = await this.previewServer.getLogs(50)
        }
      } catch (logError) {
        this.logger.debug('Could not retrieve server logs', {
          error: logError instanceof Error ? logError.message : String(logError),
        })
      }

      // Save error metadata to disk for later retrieval
      const errorMetadata: PreviewEnvironmentMetadata = {
        taskId: this.taskId,
        pid: -1, // No valid PID for failed startup
        port: -1, // No valid port for failed startup
        startTime: new Date(),
        command: serverCommand,
        workspacePath: this.workspacePath,
        logFile: '',
        processStartTime: Date.now(),
        envVars: this.repositoryConfig.codeExecution.envVars,
        failed: true,
        error: errorMessage,
        logs: logs || 'No logs available',
      }

      await this.savePreviewEnvironmentMetadata(this.taskId, errorMetadata)

      // Return failed state instead of throwing
      return {
        running: false,
        configured: true,
        uptimeSeconds: 0,
        error: errorMessage,
        logs: logs || 'No logs available',
      }
    }
  }

  async stopServer(): Promise<boolean> {
    if (!this.previewServer) {
      this.logger.debug('No server handler found')
      // Check if we have metadata for a potentially orphaned process
      const metadata = await this.loadPreviewEnvironmentMetadata(this.taskId)
      if (metadata) {
        this.logger.info('Found orphaned server metadata, attempting cleanup', {
          pid: metadata.pid,
        })
        await this.killOrphanedProcess(metadata.pid, false)
        await this.deletePreviewEnvironmentMetadata(this.taskId)
        return true
      }
      return false
    }

    try {
      // Stop proxy server first
      if (this.previewProxy) {
        try {
          await this.previewProxy.stop()
          this.previewProxy = undefined
          this.logger.info('Preview proxy stopped successfully')
        } catch (error) {
          this.logger.error('Error stopping preview proxy', error as Error)
        }
      }

      // Stop the main server
      const stopped = await this.previewServer.stop()
      if (stopped) {
        this.previewServer = undefined
        await this.deletePreviewEnvironmentMetadata(this.taskId)
        this.logger.info('Preview server stopped successfully')
      }
      return stopped
    } catch (error) {
      this.logger.error('Error stopping preview server', error as Error)
      // Force cleanup
      this.previewServer = undefined
      this.previewProxy = undefined
      await this.deletePreviewEnvironmentMetadata(this.taskId)
      return false
    }
  }

  async getServerStatus(): Promise<ServerStatus> {
    const configured = Boolean(this.repositoryConfig.previewConfig?.serverStartCommand)

    // Check running servers
    if (this.previewServer) {
      const status = this.previewServer.getStatus(configured)

      // Perform ping check if server is running
      if (status.running && status.url) {
        const port = this.extractPortFromUrl(status.url)
        const responsive = port ? await this.pingServer(port) : false

        // Get proxy URL from metadata if available
        const metadata = await this.loadPreviewEnvironmentMetadata(this.taskId)
        const proxyUrl = metadata?.proxyUrl

        return {
          ...status,
          responsive,
          proxyUrl,
        } as ServerStatus
      }

      // Get proxy URL from metadata if available for non-responsive servers
      const metadata = await this.loadPreviewEnvironmentMetadata(this.taskId)
      return {
        ...status,
        proxyUrl: metadata?.proxyUrl,
      } as ServerStatus
    }

    // Check for orphaned server via metadata
    const metadata = await this.loadPreviewEnvironmentMetadata(this.taskId)
    if (metadata) {
      // Check if this is error metadata from a failed startup
      if (metadata.failed) {
        return {
          running: false,
          configured,
          uptimeSeconds: 0,
          error: metadata.error,
          logs: metadata.logs,
        } as ServerStatus
      }

      const validation = await this.validateProcess(metadata.pid, metadata.command)
      if (validation.exists && validation.isCorrectProcess) {
        // Process is still running, recover it
        this.previewServer = new PreviewServer(
          this.taskId,
          metadata.workspacePath,
          metadata.command,
          metadata.envVars || {},
        )

        this.restoreServerHandler(this.previewServer, metadata)

        const responsive = await this.pingServer(metadata.port)
        const uptime = Math.floor((Date.now() - metadata.startTime.getTime()) / 1000)

        return {
          running: true,
          configured,
          url: `http://localhost:${metadata.port}`,
          proxyUrl: metadata.proxyUrl,
          pid: metadata.pid,
          uptimeSeconds: uptime,
          responsive,
        } as ServerStatus
      }
      // Clean up invalid metadata
      await this.deletePreviewEnvironmentMetadata(this.taskId)
    }

    return {
      running: false,
      configured,
      uptimeSeconds: 0,
    }
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (this.previewServer) {
      return await this.previewServer.getLogs(lines)
    }

    // Try to get logs from metadata
    const metadata = await this.loadPreviewEnvironmentMetadata(this.taskId)

    // Check for logs in failed metadata
    if (metadata?.logs) {
      return metadata.logs
    }

    // Check for log file
    if (metadata?.logFile) {
      try {
        const content = await fs.readFile(metadata.logFile, 'utf-8')
        const allLines = content.split('\n')
        return allLines.slice(-lines).join('\n')
      } catch (_error) {
        return 'Failed to read log file'
      }
    }
    return 'No server logs available'
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
      agentMode?: 'edit' | 'plan'
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
        agentMode: options?.agentMode,
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

  // ===============================
  // Preview Server Management
  // ===============================

  private async initializeStorage(): Promise<void> {
    try {
      await fs.mkdir(this.previewEnvironmentsPath, { recursive: true })
      // Load and validate existing preview environment metadata on startup
      await this.loadAndValidateExistingPreviewEnvironments()
    } catch (error) {
      this.logger.error('Failed to initialize preview environment storage', error as Error)
    }
  }

  /**
   * Load existing preview environment metadata and validate processes on service startup
   */
  private async loadAndValidateExistingPreviewEnvironments(): Promise<void> {
    try {
      const files = await fs.readdir(this.previewEnvironmentsPath)
      const jsonFiles = files.filter((f) => f.endsWith('.json'))

      for (const file of jsonFiles) {
        const taskId = file.replace('.json', '')
        // Only load metadata for this specific task
        if (taskId !== this.taskId) {
          continue
        }

        try {
          const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
          if (metadata) {
            // Validate if the process is still running
            const validation = await this.validateProcess(metadata.pid, metadata.command)

            if (validation.exists && validation.isCorrectProcess) {
              this.logger.info('Recovered running preview environment server', {
                taskId,
                pid: metadata.pid,
                port: metadata.port,
              })

              // Create a PreviewServer instance for the recovered server
              this.previewServer = new PreviewServer(
                taskId,
                metadata.workspacePath,
                metadata.command,
                metadata.envVars || {},
              )

              // Restore the handler's state
              this.restoreServerHandler(this.previewServer, metadata)
            } else {
              this.logger.info('Cleaning up orphaned preview environment metadata', {
                taskId,
                pid: metadata.pid,
                validation,
              })
              await this.deletePreviewEnvironmentMetadata(taskId)
            }
          }
        } catch (error) {
          this.logger.error(
            `Failed to validate preview environment for task ${taskId}`,
            error as Error,
          )
        }
      }
    } catch (error) {
      this.logger.error('Failed to load existing preview environments', error as Error)
    }
  }

  /**
   * Validate if a process is still running and is the correct one
   */
  private async validateProcess(pid: number, expectedCommand: string): Promise<ProcessValidation> {
    try {
      if (process.platform === 'win32') {
        // Windows: Use wmic to get process info
        try {
          const result = execSync(
            `wmic process where ProcessId=${pid} get CommandLine,CreationDate /format:list`,
            { encoding: 'utf-8' },
          )

          const lines = result.split('\n').filter((line) => line.trim())
          const commandLine = lines.find((line) => line.startsWith('CommandLine='))

          if (commandLine) {
            const command = commandLine.split('=')[1]
            return {
              exists: true,
              isCorrectProcess: command.includes(expectedCommand.split(' ')[0]),
              command,
            }
          }
        } catch {
          return { exists: false, isCorrectProcess: false }
        }
      } else {
        // Unix/Linux/Mac: Use ps to get process info
        try {
          const result = execSync(`ps -p ${pid} -o command=`, { encoding: 'utf-8' })
          const command = result.trim()

          return {
            exists: true,
            isCorrectProcess: command.includes(expectedCommand.split(' ')[0]),
            command,
          }
        } catch {
          return { exists: false, isCorrectProcess: false }
        }
      }
    } catch (error) {
      this.logger.debug('Error validating process', { pid, error })
      return { exists: false, isCorrectProcess: false }
    }

    return { exists: false, isCorrectProcess: false }
  }

  /**
   * Extract port number from URL string
   */
  private extractPortFromUrl(url?: string): number | null {
    if (!url) return null

    try {
      // Handle both formats: "http://localhost:port" and "localhost:port"
      const parts = url.split(':')

      if (parts.length >= 2) {
        // If URL starts with http://, port is at index 2
        // If URL is just localhost:port, port is at index 1
        const portString = url.startsWith('http://') ? parts[2] : parts[parts.length - 1]
        const port = Number.parseInt(portString, 10)

        // Validate port range
        if (Number.isNaN(port) || port < 0 || port > 65535) {
          return null
        }

        return port
      }
    } catch (error) {
      this.logger.debug('Error extracting port from URL', { url, error })
    }

    return null
  }

  /**
   * Ping a server to check if it's responsive
   */
  private async pingServer(port: number, timeout = 1000): Promise<boolean> {
    return new Promise((resolve) => {
      const client = new net.Socket()
      client.setTimeout(timeout)

      client.on('connect', () => {
        client.destroy()
        resolve(true)
      })

      client.on('timeout', () => {
        client.destroy()
        resolve(false)
      })

      client.on('error', () => {
        resolve(false)
      })

      client.connect(port, '127.0.0.1')
    })
  }

  /**
   * Kill an orphaned process
   */
  private async killOrphanedProcess(pid: number, force = false): Promise<void> {
    try {
      if (process.platform === 'win32') {
        execSync(`taskkill ${force ? '/F' : ''} /PID ${pid}`)
      } else {
        process.kill(pid, force ? 'SIGKILL' : 'SIGTERM')
      }
    } catch (error) {
      this.logger.debug('Failed to kill orphaned process', { pid, error })
    }
  }

  /**
   * Restore PreviewServer state from metadata
   */
  private restoreServerHandler(handler: PreviewServer, metadata: PreviewEnvironmentMetadata): void {
    // Use private property access to restore state
    // This is a bit hacky but necessary for recovery
    ;(handler as unknown as { port: number }).port = metadata.port
    ;(handler as unknown as { pid: number }).pid = metadata.pid
    ;(handler as unknown as { logFile: string }).logFile = metadata.logFile
    ;(handler as unknown as { startTime: number }).startTime = metadata.startTime.getTime()

    // Create a mock process object with event emitter functionality
    const mockProcess = {
      exitCode: null,
      on: (_event: string, _callback: (...args: unknown[]) => void) => {},
      kill: (_signal?: string) => {},
    }
    ;(handler as unknown as { process: typeof mockProcess }).process = mockProcess
  }

  /**
   * Save preview environment metadata to disk
   */
  private async savePreviewEnvironmentMetadata(
    taskId: string,
    metadata: PreviewEnvironmentMetadata,
  ): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    await fs.writeFile(filePath, JSON.stringify(metadata, null, 2), 'utf-8')
  }

  /**
   * Load preview environment metadata from disk
   */
  private async loadPreviewEnvironmentMetadata(
    taskId: string,
  ): Promise<PreviewEnvironmentMetadata | null> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    try {
      const content = await fs.readFile(filePath, 'utf-8')
      const data = JSON.parse(content)
      // Convert date strings back to Date objects
      data.startTime = new Date(data.startTime)
      return data
    } catch {
      return null
    }
  }

  /**
   * Delete preview environment metadata from disk
   */
  private async deletePreviewEnvironmentMetadata(taskId: string): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    try {
      await fs.unlink(filePath)
    } catch {
      // Ignore errors if file doesn't exist
    }
  }
}
