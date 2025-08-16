import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { CommandResult, RepositoryConfig, ServerStatus } from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import { ProxyAgent } from '../agents/proxy-agent.js'
import type { DockerService } from '../services/docker.service.js'
import type { EventService } from '../services/event.service.js'
import { createTaskLogger } from '../utils/logger.js'
import { BaseEnvironment } from './base-environment.js'

const execAsync = promisify(exec)

export class DockerEnvironment extends BaseEnvironment {
  private image = ''
  private _createdAt = new Date()
  private _error?: string
  private workspacePath: string
  private workerPortMapping?: { containerPort: number; hostPort: number }
  private logger = createTaskLogger(this.taskId)

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath: string,
    private dockerService: DockerService,
    private claudeService: ClaudeService,
    eventService?: EventService,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.workspacePath = workspacePath
    this.logger = createTaskLogger(taskId)
  }

  getType(): 'docker' {
    return 'docker'
  }

  getStatus(): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    return this.getContainerStatus()
  }

  private getContainerStatus(): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    // Always generate the container name deterministically
    const containerName = this.getContainerName()
    return this.dockerService.getContainerStatus(containerName)
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

  getContainerName(): string {
    return this.dockerService.generateContainerName(this.repositoryConfig.name, this.taskId)
  }

  getPort(): number | undefined {
    return this.workerPortMapping?.hostPort
  }

  getImage(): string {
    return this.image
  }

  async setup(_iterationNumber?: number): Promise<void> {
    const previousStatus = this.getStatus()
    const startTime = Date.now()
    const containerName = this.getContainerName()

    this.logger.info('Starting Docker environment setup', {
      taskId: this.taskId,
      workspacePath: this.workspacePath,
      repositoryName: this.repositoryConfig.name,
    })

    // Emit event for initializing status
    await this.emitStatusChange('initializing', previousStatus)

    // Write repository configuration to a temporary file
    const tmpDir = path.join(process.env.TMPDIR || '/tmp', 'moatless-configs')
    await fs.mkdir(tmpDir, { recursive: true })
    const configPath = path.join(tmpDir, `repository-config-${this.taskId}.json`)

    try {
      await fs.writeFile(configPath, JSON.stringify(this.repositoryConfig, null, 2))
      this.logger.info('Wrote repository configuration to temp file', { configPath })
    } catch (error) {
      this.logger.error('Failed to write repository configuration', error as Error)
      throw error
    }

    // Validate Claude authentication using ClaudeService
    const claudeAuth = this.claudeService.validateClaudeAuthentication()
    if (!claudeAuth.isValid && claudeAuth.error) {
      if (claudeAuth.hasConfig && !claudeAuth.hasCredentials && !claudeAuth.hasOAuthToken) {
        // This is a critical error - fail setup
        this.logger.error(claudeAuth.error)
        throw new Error(claudeAuth.error)
      }
      // This is a warning - continue but log
      this.logger.warn(claudeAuth.error)
    }

    try {
      // Allocate port for worker using DockerService
      this.workerPortMapping = await this.dockerService.allocateWorkerPort()

      this.image =
        this.repositoryConfig.codeExecution.dockerImage || 'moatless-vibe-universal:latest'

      this.logger.info('Creating Docker container', {
        containerName: containerName,
        image: this.image,
        workerPort: this.workerPortMapping.hostPort,
      })

      await this.createOrReuseContainer()

      this.logger.info('Starting Docker container', {
        containerName: containerName,
      })

      await this.startContainer()

      // Copy repository configuration to container BEFORE waiting for worker
      // The worker needs this config to start successfully
      try {
        // Copy the config file (directory already exists in image)
        await execAsync(`docker cp "${configPath}" "${containerName}:/data/repository-config.json"`)
        this.logger.info('Repository configuration copied to container')

        // Clean up temp file
        await fs.unlink(configPath)
      } catch (error) {
        this.logger.error('Failed to copy repository configuration', error as Error)
        throw error // This is critical - worker cannot start without it
      }

      // Wait for the container and worker to be ready
      this.logger.info('Waiting for container to be ready', {
        containerName: containerName,
      })

      await this.waitForContainerReady()

      // Copy Claude config files if they exist using ClaudeService
      const claudePaths = this.claudeService.getClaudeConfigPaths()
      if (claudePaths.configPath) {
        this.logger.info('Copying Claude config to container', {
          configPath: claudePaths.configPath,
        })
        await execAsync(
          `docker cp "${claudePaths.configPath}" "${containerName}:/home/ubuntu/.claude/.claude.json"`,
        )

        // Fix ownership of entire .claude directory and its contents to ubuntu user (run as root)
        await execAsync(
          `docker exec --user root "${containerName}" chown -R ubuntu:ubuntu /home/ubuntu/.claude`,
        )
        this.logger.info('Claude config copied and ownership fixed successfully')
      } else {
        this.logger.info('No Claude config files to copy - skipping Claude setup')
      }

      // Emit event for ready status
      await this.emitStatusChange('ready', 'initializing')

      this.logger.info('DockerEnvironment setup completed - container is ready', {
        containerName: containerName,
        workerPort: this.workerPortMapping.hostPort,
        setupTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      this._error = error instanceof Error ? error.message : String(error)

      // Emit event for error status
      await this.emitStatusChange(
        'error',
        'initializing',
        error instanceof Error ? error : String(error),
      )

      this.logger.error('DockerEnvironment setup failed', error as Error, {
        setupTimeMs: failureTime,
        containerName: containerName,
      })

      await this.cleanup()
      throw error
    }
  }

  private async createOrReuseContainer(): Promise<void> {
    const containerName = this.getContainerName()

    // Check if container already exists using DockerService
    const existingContainer = await this.dockerService.getExistingContainer(containerName)

    if (existingContainer) {
      this.logger.info('Found existing container', {
        containerName: containerName,
        status: existingContainer.status,
      })

      // If container is running, reuse it
      if (existingContainer.status.includes('Up')) {
        this.logger.info('Container is running, reusing it')
        return
      }

      // If container is stopped, remove it and create a new one
      this.logger.info('Container is stopped, removing it and creating new one')
      await this.dockerService.removeExistingContainer(existingContainer.id)
    }

    // No existing container found or removed stopped container, create a new one
    this.logger.info('Creating new container')
    await this.createContainer()
  }

  private async createContainer(): Promise<void> {
    const containerName = this.getContainerName()

    if (!this.workerPortMapping) {
      throw new Error('Worker port not allocated')
    }

    // Use DockerService to create container with all configuration
    await this.dockerService.createContainer(
      containerName,
      this.taskId,
      this.repositoryConfig,
      this.workspacePath,
      this.workerPortMapping,
      this.image,
    )
  }

  private async startContainer(): Promise<void> {
    const containerName = this.getContainerName()
    await this.dockerService.startContainer(containerName)
  }

  /**
   * Override to implement custom error recovery for Docker containers
   */
  protected async recoverFromError(): Promise<void> {
    this.logger.info('Container in error state, attempting recovery', {
      previousError: this._error,
      containerName: this.getContainerName(),
    })

    // Clean up the failed container
    await this.cleanup()

    // Reset error state
    this._error = undefined

    // Setup container
    await this.setup()
  }

  /**
   * Override to implement Docker-specific waiting logic
   */
  protected async waitForReady(): Promise<void> {
    const status = this.getStatus()

    if (status === 'cleaning') {
      // Container is being removed, wait for it to finish then check again
      this.logger.info('Container is being cleaned up, waiting for completion')
      await new Promise((resolve) => setTimeout(resolve, 2000))
      // Recursively check status again after cleanup
      return this.ensureReady()
    }

    if (status === 'initializing') {
      // Already in progress, just need to wait
      this.logger.info('Container setup in progress, waiting for completion', { status })
      await this.waitForContainerReady()
    }
  }

  /**
   * Waits for the container to be ready for command execution or agent creation.
   */
  private async waitForContainerReady(): Promise<void> {
    const maxWaitTime = 180000 // 3 minute timeout for waiting
    const checkInterval = 2000 // 2 seconds
    let elapsed = 0

    const containerName = this.getContainerName()

    this.logger.debug('Waiting for container to become ready', {
      containerName: containerName,
      workerPort: this.workerPortMapping?.hostPort,
      maxWaitTime,
    })

    // First, ensure we have worker port mapping
    if (!this.workerPortMapping) {
      throw new Error('Worker port mapping not available - container setup incomplete')
    }

    while (elapsed < maxWaitTime) {
      const status = this.getStatus()

      if (status === 'ready') {
        this.logger.info('Container is ready, wait for worker readiness', {
          elapsedMs: elapsed,
          containerName: containerName,
          workerPort: this.workerPortMapping.hostPort,
        })

        while (elapsed < maxWaitTime) {
          try {
            const response = await fetch(
              `http://localhost:${this.workerPortMapping?.hostPort}/health`,
            )
            if (response.ok) {
              this.logger.info('Worker is ready', {
                elapsedMs: elapsed,
                workerPort: this.workerPortMapping?.hostPort,
              })
              return
            }
          } catch (_error) {
            // Worker not ready yet
            if (elapsed % 5000 === 0) {
              // Log every 5 seconds
              this.logger.debug('Worker not ready yet, continuing to wait', {
                elapsedMs: elapsed,
                remainingMs: maxWaitTime - elapsed,
              })
            }
          }

          await new Promise((resolve) => setTimeout(resolve, checkInterval))
          elapsed += checkInterval
        }

        throw new Error(`Worker did not become ready within timeout (${maxWaitTime}ms)`)
      }

      if (status === 'error') {
        const error = this.getError() || 'Container failed'
        throw new Error(error)
      }

      if (status === 'not_created') {
        throw new Error('Container lost during wait - setup may have failed')
      }

      // Log progress every 10 seconds
      if (elapsed % 10000 === 0 && elapsed > 0) {
        this.logger.debug('Container still starting', {
          elapsedMs: elapsed,
          remainingMs: maxWaitTime - elapsed,
          containerName: containerName,
        })
      }

      await new Promise((resolve) => setTimeout(resolve, checkInterval))
      elapsed += checkInterval
    }

    throw new Error(`Container did not become ready imeout (${maxWaitTime}ms)`)
  }

  async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
    // Ensure container is ready before executing command
    await this.ensureReady()

    // Double-check we have worker port mapping
    if (!this.workerPortMapping) {
      throw new Error('Worker port mapping not available - container setup incomplete')
    }

    const containerName = this.getContainerName()
    this.logger.debug(`Running command via docker exec: ${command}`)

    const startTime = Date.now()

    try {
      // Use DockerService to execute command in container
      const { stdout, stderr } = await this.dockerService.executeCommandInContainer(
        containerName,
        command,
        this.repositoryConfig,
        this.workspacePath,
        timeout,
      )

      return {
        exitCode: 0,
        output: stdout + stderr,
        durationMs: Date.now() - startTime,
        command,
      }
    } catch (error) {
      this.logger.error('Command execution failed', error as Error)

      // Extract exit code from error if available
      let exitCode = 1
      let output = ''

      if (error instanceof Error) {
        output = error.message
        // Check if error has exit code property (from child_process)
        if ('code' in error && typeof error.code === 'number') {
          exitCode = error.code
        }
        // Extract stdout/stderr from exec error
        if ('stdout' in error && typeof error.stdout === 'string') {
          output = error.stdout
        }
        if ('stderr' in error && typeof error.stderr === 'string') {
          output += error.stderr
        }
      }

      return {
        exitCode,
        output,
        durationMs: Date.now() - startTime,
        command,
      }
    }
  }

  async startServer(): Promise<ServerStatus> {
    // Ensure container is ready before starting server
    await this.ensureReady()

    this.logger.info('Starting server via worker')

    // Get server configuration from repository config
    const serverCommand = this.repositoryConfig.previewConfig?.serverStartCommand
    if (!serverCommand) {
      throw new Error('No server command configured for this environment')
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/start`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            serverCommand,
            envVars: this.repositoryConfig.codeExecution.envVars || {},
          }),
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
      containerName: this.getContainerName(),
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
    // Check if container is ready before getting server status
    const status = this.getStatus()
    if (status !== 'ready') {
      return {
        running: false,
        configured: !!this.repositoryConfig.previewConfig?.serverStartCommand,
        uptimeSeconds: 0,
      }
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/status`,
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
    // Check if container is ready before getting server logs
    const status = this.getStatus()
    if (status !== 'ready') {
      return 'Container not ready - no logs available'
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/logs?lines=${lines}`,
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

  getPreviewPort(): number | undefined {
    return undefined // Preview port managed by worker
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
    // Ensure container is ready before proxying request
    await this.ensureReady()

    try {
      // Proxy the request through the worker which will forward to the preview server
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/proxy${request.path}`,
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

      if (
        contentType.includes('text/') ||
        contentType.includes('application/json') ||
        contentType.includes('application/javascript')
      ) {
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

  async createAgent(agentType: string, options?: AgentOptions): Promise<BaseAgent> {
    // Ensure container is ready before creating agent
    await this.ensureReady()

    // Double-check we have worker port mapping
    if (!this.workerPortMapping) {
      throw new Error('Worker port mapping not available - container setup incomplete')
    }

    // Return a proxy agent that delegates to the worker
    return new ProxyAgent({
      taskId: this.taskId,
      agentType,
      workerPort: this.workerPortMapping.hostPort,
      options,
      logger: this.logger,
    })
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
        workspaceDir: options?.workspaceDir || this.repositoryConfig.path,
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
      // Make HTTP request to worker to stop agent
      const workerUrl = `http://localhost:${this.workerPortMapping?.hostPort}/stop-agent`

      const response = await fetch(workerUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId: this.taskId }),
      })

      if (!response.ok) {
        const error = await response.text()
        this.logger.error('Failed to stop agent in worker', new Error(error))
      }

      this.currentAgent = undefined
      return true
    } catch (error) {
      this.logger.error('Error stopping agent', error as Error)
      this.currentAgent = undefined
      return false
    }
  }

  async teardown(force = false): Promise<void> {
    this.logger.info('Tearing down DockerEnvironment', {
      force,
      containerName: this.getContainerName(),
    })

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

      this.logger.info('DockerEnvironment teardown completed')
    } catch (error) {
      this.logger.error('Error during teardown', error as Error)
      if (!force) {
        throw error
      }
    }
  }

  private async cleanup(): Promise<void> {
    const containerName = this.getContainerName()

    // Check if container exists before trying to clean it up
    const previousStatus = this.getStatus()
    if (previousStatus === 'not_created') {
      return
    }

    // Emit event for cleaning status
    await this.emitStatusChange('cleaning', previousStatus)

    // Use DockerService to stop and remove container
    await this.dockerService.stopContainer(containerName)
    await this.dockerService.removeContainer(containerName)
  }
}
