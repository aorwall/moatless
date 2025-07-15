import { exec } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { RepositoryConfig } from '@moatless/types'
import type { CommandResult, ServerStatus } from '@moatless/types'
import type { BaseAgent, EventService } from '../services/index.js'
import { PortAllocationService } from '../services/port-allocation.service.js'
import { createTaskLogger } from '../utils/logger.js'
import { BaseEnvironment } from './base-environment.js'

const execAsync = promisify(exec)

export class DockerEnvironment extends BaseEnvironment {
  private containerId = ''
  private containerName = ''
  private image = ''
  private setupInProgress = false
  private setupComplete = false
  private runningAgent?: BaseAgent
  private _createdAt = new Date()
  private _error?: string
  private workspacePath: string
  private previewPortMapping?: { containerPort: number; hostPort: number }
  private workerPortMapping?: { containerPort: number; hostPort: number }
  private logger = createTaskLogger(this.taskId)
  private portAllocationService: PortAllocationService

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    eventService: EventService,
    workspacePath: string,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.workspacePath = workspacePath
    this.portAllocationService = PortAllocationService.getInstance()
    this.logger = createTaskLogger(taskId)
  }

  getType(): 'docker' {
    return 'docker'
  }

  getStatus(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) return 'error'
    if (this.setupInProgress) return 'initializing'
    if (this.setupComplete) return 'ready'
    return 'not_setup'
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

  getContainerId(): string {
    return this.containerId
  }

  async setup(_iterationNumber?: number): Promise<void> {
    this.setupInProgress = true
    const startTime = Date.now()

    try {
      // Initialize port allocation service
      await this.portAllocationService.initialize()

      // Allocate ports for worker and preview server
      const workerPort = await this.portAllocationService.allocatePort(this.taskId, 'worker')
      const previewPort = await this.portAllocationService.allocatePort(this.taskId, 'preview')

      this.workerPortMapping = { containerPort: 8001, hostPort: workerPort }
      this.previewPortMapping = { containerPort: 3000, hostPort: previewPort }

      this.image =
        this.repositoryConfig.codeExecution.dockerImage || 'moatless-vibe-universal:latest'

      // Create container
      this.containerName = this.repositoryConfig.codeExecution.containerName
        ? `${this.repositoryConfig.codeExecution.containerName}-${this.taskId}`
        : `moatless-task-${this.taskId}`

      await this.createContainer()
      await this.startContainer()

      // Wait for worker to be ready
      await this.waitForWorkerReady()

      this.setupComplete = true
      this.setupInProgress = false

      this.logger.info('DockerEnvironment setup completed', {
        containerId: this.containerId,
        workerPort: this.workerPortMapping.hostPort,
        previewPort: this.previewPortMapping.hostPort,
        setupTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      this._error = error instanceof Error ? error.message : String(error)

      this.logger.error('DockerEnvironment setup failed', error as Error, {
        setupTimeMs: failureTime,
        containerId: this.containerId,
      })

      this.setupInProgress = false
      await this.cleanup()
      throw error
    }
  }

  private async createContainer(): Promise<void> {
    let createCommand = `docker create --name "${this.containerName}"`

    // Add moatless labels for easy cleanup
    createCommand += ` --label "created-by=moatless"`
    createCommand += ` --label "moatless.task-id=${this.taskId}"`
    createCommand += ` --label "moatless.workspace-type=docker"`
    createCommand += ` --label "moatless.created-at=${new Date().toISOString()}"`

    // Set working directory to /repo
    createCommand += ` -w "/repo"`

    // Add volume mappings
    const dataDir = process.env.DATA_DIR || './data'
    createCommand += ` -v "${this.workspacePath}:/repo"`
    createCommand += ` -v "${dataDir}:/data"`

    // Mount Claude config directory
    const containerClaudeConfigDir = '/home/node/.claude'
    const hostClaudeConfigDir =
      process.env.CLAUDE_CONFIG_DIR || path.join(process.env.HOME || '', '.claude')

    if (hostClaudeConfigDir && existsSync(hostClaudeConfigDir)) {
      createCommand += ` -v "${hostClaudeConfigDir}:${containerClaudeConfigDir}"`
    }

    // Add port mappings
    if (this.workerPortMapping) {
      createCommand += ` -p ${this.workerPortMapping.hostPort}:${this.workerPortMapping.containerPort}`
    }
    if (this.previewPortMapping) {
      createCommand += ` -p ${this.previewPortMapping.hostPort}:${this.previewPortMapping.containerPort}`
    }

    // Add environment variables
    const port = process.env.PORT || '8000'
    const apiUrl =
      process.env.WORKER_API_URL || process.env.API_URL || `http://host.docker.internal:${port}`
    const apiKey = process.env.WORKER_API_KEY || process.env.API_KEY || 'test-key'

    createCommand += ` -e "WORKER_MODE=true"`
    createCommand += ` -e "WORKER_API_URL=${apiUrl}"`
    createCommand += ` -e "WORKER_API_KEY=${apiKey}"`
    createCommand += ` -e "WORKER_TASK_ID=${this.taskId}"`
    createCommand += ` -e "WORKSPACE_DIR=/repo"`
    createCommand += ` -e "DATA_DIR=/data"`
    createCommand += ` -e "PORT=${this.workerPortMapping?.containerPort || 8001}"`
    createCommand += ` -e "PREVIEW_PORT=${this.previewPortMapping?.containerPort || 3000}"`
    createCommand += ` -e "LOG_LEVEL=debug"`

    // Pass through Claude authentication environment variables
    if (process.env.CLAUDE_AUTH_DIR) {
      createCommand += ` -e "CLAUDE_AUTH_DIR=${process.env.CLAUDE_AUTH_DIR}"`
    }
    if (process.env.ANTHROPIC_API_KEY) {
      createCommand += ` -e "ANTHROPIC_API_KEY=${process.env.ANTHROPIC_API_KEY}"`
    }
    createCommand += ` -e "CLAUDE_CONFIG_DIR=${containerClaudeConfigDir}"`
    createCommand += ` -e "HOME=/home/node"`

    if (process.env.CLAUDE_CODE_MOCK) {
      createCommand += ` -e "CLAUDE_CODE_MOCK=${process.env.CLAUDE_CODE_MOCK}"`
    }

    // Add repository-specific environment variables
    const envVars = this.repositoryConfig.codeExecution.envVars || {}
    for (const [key, value] of Object.entries(envVars)) {
      createCommand += ` -e "${key}=${value}"`
    }

    createCommand += ` ${this.image}`

    this.logger.debug('Creating container', { createCommand })

    try {
      const { stdout } = await execAsync(createCommand)
      this.containerId = stdout.trim()
      this.logger.info('Container created', { containerId: this.containerId })
    } catch (error) {
      this.logger.error('Failed to create container', error as Error)
      throw error
    }
  }

  private async startContainer(): Promise<void> {
    this.logger.info('Starting container', { containerId: this.containerId })

    try {
      await execAsync(`docker start ${this.containerId}`)
      this.logger.info('Container started', { containerId: this.containerId })
    } catch (error) {
      this.logger.error('Failed to start container', error as Error)
      throw error
    }
  }

  private async waitForWorkerReady(): Promise<void> {
    const maxWaitTime = 30000 // 30 seconds
    const checkInterval = 1000 // 1 second
    let elapsed = 0

    while (elapsed < maxWaitTime) {
      try {
        const response = await fetch(`http://localhost:${this.workerPortMapping?.hostPort}/health`)
        if (response.ok) {
          this.logger.info('Worker is ready')
          return
        }
      } catch {
        // Worker not ready yet
      }

      await new Promise((resolve) => setTimeout(resolve, checkInterval))
      elapsed += checkInterval
    }

    throw new Error('Worker did not become ready within timeout')
  }

  async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
    if (!this.workerPortMapping) {
      throw new Error('Worker port not allocated')
    }

    this.logger.debug(`Running command via worker: ${command}`)

    try {
      const response = await fetch(`http://localhost:${this.workerPortMapping.hostPort}/command`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ command, timeout }),
      })

      if (!response.ok) {
        throw new Error(`Worker command failed: ${response.statusText}`)
      }

      const result = (await response.json()) as {
        exit_code: number
        stdout: string
        stderr: string
      }
      return {
        exitCode: result.exit_code,
        output: result.stdout,
        durationMs: 0,
        command,
      }
    } catch (error) {
      this.logger.error('Command execution failed', error as Error)
      return {
        exitCode: 1,
        output: '',
        durationMs: 0,
        command,
      }
    }
  }

  async startServer(): Promise<ServerStatus> {
    if (!this.workerPortMapping) {
      throw new Error('Worker port not allocated')
    }

    this.logger.info('Starting server via worker')

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping.hostPort}/server/start`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
        },
      )

      if (!response.ok) {
        throw new Error(`Failed to start server: ${response.statusText}`)
      }

      const result = (await response.json()) as ServerStatus

      // For proxy approach, we don't expose the direct port
      // The preview will be accessed through the proxy endpoint
      if (result.running) {
        result.url = undefined // Frontend will use proxy endpoint
      }

      return result
    } catch (error) {
      this.logger.error('Failed to start server', error as Error)
      throw error
    }
  }

  async stopServer(): Promise<boolean> {
    if (!this.workerPortMapping) {
      return false
    }

    this.logger.info('Stopping server via worker', {
      workerPort: this.workerPortMapping.hostPort,
      containerId: this.containerId,
    })

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping.hostPort}/server/stop`,
        {
          method: 'POST',
          signal: AbortSignal.timeout(5000), // 5 second timeout
        },
      )

      if (!response.ok) {
        // Handle 404 gracefully - server may already be stopped
        if (response.status === 404) {
          this.logger.debug('Server stop endpoint not found - server may already be stopped')
          return true
        }
        throw new Error(`Failed to stop server: ${response.statusText}`)
      }

      const result = (await response.json()) as { success: boolean }
      return result.success
    } catch (error) {
      // Handle connection errors gracefully during shutdown
      if (error instanceof Error) {
        if (error.message.includes('ECONNREFUSED') || error.message.includes('ENOTFOUND')) {
          this.logger.debug('Worker container not reachable - treating as already stopped')
          return true
        }
      }
      this.logger.error('Failed to stop server', error as Error)
      return false
    }
  }

  async getServerStatus(): Promise<ServerStatus> {
    if (!this.workerPortMapping) {
      return {
        running: false,
        configured: !!this.repositoryConfig.previewConfig?.serverStartCommand,
        uptimeSeconds: 0,
      }
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping.hostPort}/server/status`,
      )

      if (!response.ok) {
        throw new Error(`Failed to get server status: ${response.statusText}`)
      }

      const result = (await response.json()) as ServerStatus

      // For proxy approach, we don't expose the direct port
      // The preview will be accessed through the proxy endpoint
      if (result.running) {
        result.url = undefined // Frontend will use proxy endpoint
      }

      return result
    } catch (error) {
      this.logger.error('Failed to get server status', error as Error)
      return {
        running: false,
        configured: !!this.repositoryConfig.previewConfig?.serverStartCommand,
        uptimeSeconds: 0,
      }
    }
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (!this.workerPortMapping) {
      return 'No worker running'
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping.hostPort}/server/logs?lines=${lines}`,
      )

      if (!response.ok) {
        throw new Error(`Failed to get server logs: ${response.statusText}`)
      }

      const result = (await response.json()) as { logs: string }
      return result.logs
    } catch (error) {
      this.logger.error('Failed to get server logs', error as Error)
      return `Error getting logs: ${error}`
    }
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
    if (!this.workerPortMapping || !this.previewPortMapping) {
      throw new Error('Preview server not running')
    }

    try {
      // Proxy the request through the worker which will forward to the preview server
      const response = await fetch(
        `http://localhost:${this.workerPortMapping.hostPort}/server/proxy${request.path}`,
        {
          method: request.method,
          headers: request.headers,
          body: request.body,
        },
      )

      const responseHeaders = new Headers()
      response.headers.forEach((value, key) => {
        responseHeaders.set(key, value)
      })

      // Read response body
      const contentType = response.headers.get('content-type') || ''
      let body: string | Buffer

      if (contentType.includes('text/') || contentType.includes('application/json') || contentType.includes('application/javascript')) {
        body = await response.text()
      } else {
        body = Buffer.from(await response.arrayBuffer())
      }

      return {
        status: response.status,
        headers: responseHeaders,
        body,
      }
    } catch (error) {
      this.logger.error('Failed to proxy preview request', error as Error)
      throw error
    }
  }

  async stopAgent(): Promise<boolean> {
    if (!this.runningAgent) {
      return false
    }

    this.logger.info('Stopping running agent')

    try {
      await this.runningAgent.stop()
      this.runningAgent = undefined
      return true
    } catch (error) {
      this.logger.error('Error stopping agent', error as Error)
      this.runningAgent = undefined
      return false
    }
  }

  isRunning(): boolean {
    return !!this.runningAgent
  }

  async teardown(force = false): Promise<void> {
    this.logger.info('Tearing down DockerEnvironment', { force, containerId: this.containerId })

    try {
      // Stop agent
      await this.stopAgent()

      // Stop server - don't let failures here stop the teardown
      try {
        await this.stopServer()
      } catch (serverError) {
        this.logger.warn('Error stopping server during teardown (continuing)', {
          error: (serverError as Error).message,
          stack: (serverError as Error).stack,
        })
      }

      // Cleanup container
      await this.cleanup()

      // Release ports
      await this.portAllocationService.releasePortsForTask(this.taskId)

      this.logger.info('DockerEnvironment teardown completed')
    } catch (error) {
      this.logger.error('Error during teardown', error as Error)
      if (!force) {
        throw error
      }
    }
  }

  setRunningAgent(agent: BaseAgent): void {
    this.runningAgent = agent
  }

  getRunningAgent(): BaseAgent | undefined {
    return this.runningAgent
  }

  private async cleanup(): Promise<void> {
    if (!this.containerId) {
      return
    }

    try {
      // Stop container
      await execAsync(`docker stop ${this.containerId}`)
      this.logger.info('Container stopped', { containerId: this.containerId })
    } catch (error) {
      this.logger.warn('Failed to stop container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    try {
      // Remove container
      await execAsync(`docker rm ${this.containerId}`)
      this.logger.info('Container removed', { containerId: this.containerId })
    } catch (error) {
      this.logger.warn('Failed to remove container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    this.containerId = ''
  }
}
