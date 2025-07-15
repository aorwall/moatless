import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import { promisify } from 'node:util'
import type { RepositoryConfig } from '@moatless/types'
import type { CommandResult, ServerStatus } from '@moatless/types'
import type { BaseAgent, EventService } from '../services/index.js'
import { PreviewServer } from '../services/preview-server.js'
import { createTaskLogger } from '../utils/logger.js'
import { BaseEnvironment } from './base-environment.js'

const execAsync = promisify(exec)

export class LocalEnvironment extends BaseEnvironment {
  private _runningAgent?: BaseAgent
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
    eventService: EventService,
    workspacePath?: string,
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

  getStatus(): 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) return 'error'
    if (this.setupInProgress) return 'initializing'
    if (this.setupComplete) return 'ready'
    return 'initializing'
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
    this.setupInProgress = true
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
      this.runSetupCommandsInBackground(startTime).catch((error) => {
        this._error = error instanceof Error ? error.message : String(error)
        this.logger.error('Background setup commands failed', error as Error)
      })

      this.logger.info('LocalEnvironment setup completed', {
        workspacePath: this.workspacePath,
        setupTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      this._error = error instanceof Error ? error.message : String(error)

      this.logger.error('LocalEnvironment setup failed', error as Error, {
        setupTimeMs: failureTime,
        workspacePath: this.workspacePath,
      })

      this.setupInProgress = false
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

      this.setupComplete = true
      this.setupInProgress = false

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
    } catch (error: any) {
      this.logger.error('Command execution failed', error, {
        command,
        workspacePath: this.workspacePath,
      })

      return {
        exitCode: error.code || 1,
        output: error.stdout?.trim() || error.message || '',
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

    return this.previewServer.getStatus(true)
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (!this.previewServer) {
      return 'No server running'
    }

    return this.previewServer.getLogs(lines)
  }

  async stopAgent(): Promise<boolean> {
    if (!this._runningAgent) {
      return false
    }

    this.logger.info('Stopping running agent')

    try {
      await this._runningAgent.stop()
      this._runningAgent = undefined
      return true
    } catch (error) {
      this.logger.error('Error stopping agent', error as Error)
      this._runningAgent = undefined
      return false
    }
  }

  isRunning(): boolean {
    return !!this._runningAgent
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

  setRunningAgent(agent: BaseAgent): void {
    this._runningAgent = agent
  }

  getRunningAgent(): BaseAgent | undefined {
    return this._runningAgent
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
