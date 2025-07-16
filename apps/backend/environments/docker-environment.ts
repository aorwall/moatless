import { exec, execSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { CommandResult, RepositoryConfig, ServerStatus } from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import { ProxyAgent } from '../agents/proxy-agent.js'
import type { EventService } from '../services/index.js'
import { generateContainerName } from '../utils/docker-container-name.js'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
import { BaseEnvironment } from './base-environment.js'

const execAsync = promisify(exec)

export class DockerEnvironment extends BaseEnvironment {
  private containerName = ''
  private image = ''
  private runningAgent?: BaseAgent
  private _createdAt = new Date()
  private _error?: string
  private workspacePath: string
  private workerPortMapping?: { containerPort: number; hostPort: number }
  private logger = createTaskLogger(this.taskId)

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    eventService: EventService,
    workspacePath: string,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.workspacePath = workspacePath
    this.logger = createTaskLogger(taskId)
  }

  getType(): 'docker' {
    return 'docker'
  }

  getStatus(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) return 'error'

    // If no container name, we haven't started setup yet
    if (!this.containerName) {
      return 'not_setup'
    }

    // Check container status to determine our state
    try {
      // This is a synchronous check, so we'll use the cached status approach
      // The actual container state checking will be done in a helper method
      return this.getContainerStatus()
    } catch (error) {
      this.logger.debug('Error checking container status', { error })
      return 'error'
    }
  }

  private getContainerStatus(): 'not_setup' | 'initializing' | 'ready' | 'error' {
    if (!this.containerName) {
      return 'not_setup'
    }

    // Check container status and health using docker inspect
    try {
      // Get container status and health in one command
      const inspectResult = execSync(
        `docker inspect --format='{{.State.Status}}|{{.State.Health.Status}}' "${this.containerName}"`,
        { encoding: 'utf8', timeout: 2000 },
      ).trim()

      if (!inspectResult) {
        return 'not_setup'
      }

      const [containerStatus, healthStatus] = inspectResult.split('|')

      // Check container state
      if (containerStatus !== 'running') {
        if (containerStatus === 'exited' || containerStatus === 'dead') {
          return 'error'
        }
        return 'not_setup'
      }

      // Container is running, check health status
      // Health status can be: none, starting, healthy, unhealthy
      switch (healthStatus) {
        case 'healthy':
          return 'ready'
        case 'starting':
          return 'initializing'
        case 'unhealthy':
          // Log unhealthy state for debugging
          this.logger.debug('Container health check failed', {
            containerName: this.containerName,
            healthStatus,
          })
          return 'error'
        case 'none':
        case '':
          // No health check defined (old image) - fall back to port check
          if (this.workerPortMapping) {
            try {
              execSync(
                `curl -s --connect-timeout 1 --max-time 2 http://localhost:${this.workerPortMapping.hostPort}/health > /dev/null 2>&1`,
                { timeout: 2500 },
              )
              return 'ready'
            } catch {
              return 'initializing'
            }
          }
          return 'initializing'
        default:
          this.logger.debug('Unknown health status', { healthStatus })
          return 'initializing'
      }
    } catch (error) {
      // Container doesn't exist
      if (error instanceof Error && error.message.includes('No such object')) {
        return 'not_setup'
      }
      // Other errors
      this.logger.debug('Error checking container status', { error })
      return 'error'
    }
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
    return this.containerName
  }

  getPort(): number | undefined {
    return this.workerPortMapping?.hostPort
  }

  getImage(): string {
    return this.image
  }

  async setup(_iterationNumber?: number): Promise<void> {
    const startTime = Date.now()

    this.logger.info('Starting Docker environment setup', {
      taskId: this.taskId,
      workspacePath: this.workspacePath,
      repositoryName: this.repositoryConfig.name,
    })

    try {
      // Allocate port for worker
      const workerPort = await findAvailablePort('worker')

      this.workerPortMapping = { containerPort: 8001, hostPort: workerPort }

      this.image =
        this.repositoryConfig.codeExecution.dockerImage || 'moatless-vibe-universal:latest'

      // Create container
      this.containerName = generateContainerName(this.repositoryConfig.name, this.taskId)

      this.logger.info('Creating Docker container', {
        containerName: this.containerName,
        image: this.image,
        workerPort,
      })

      await this.createOrReuseContainer()

      this.logger.info('Starting Docker container', {
        containerName: this.containerName,
      })

      await this.startContainer()

      // Wait for the container and worker to be ready
      this.logger.info('Waiting for container to be ready', {
        containerName: this.containerName,
      })

      await this.waitForContainerReady()

      this.logger.info('DockerEnvironment setup completed - container is ready', {
        containerName: this.containerName,
        workerPort: this.workerPortMapping.hostPort,
        setupTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      this._error = error instanceof Error ? error.message : String(error)

      this.logger.error('DockerEnvironment setup failed', error as Error, {
        setupTimeMs: failureTime,
        containerName: this.containerName,
      })

      await this.cleanup()
      throw error
    }
  }

  private async createOrReuseContainer(): Promise<void> {
    // Check if container already exists
    const existingContainer = await this.getExistingContainer()

    if (existingContainer) {
      this.logger.info('Found existing container', {
        containerName: this.containerName,
        status: existingContainer.status,
      })

      // If container is running, reuse it
      if (existingContainer.status.includes('Up')) {
        this.logger.info('Container is running, reusing it')
        return
      }

      // If container is stopped, remove it and create a new one
      this.logger.info('Container is stopped, removing it and creating new one')
      await this.removeExistingContainer(existingContainer.id)
    }

    // No existing container found or removed stopped container, create a new one
    this.logger.info('Creating new container')
    await this.createContainer()
  }

  private async getExistingContainer(): Promise<{ id: string; status: string } | null> {
    try {
      // Use exact name match with ^/name$ pattern to avoid partial matches
      const { stdout } = await execAsync(
        `docker ps -a --filter "name=^/${this.containerName}$" --format "{{.ID}},{{.Status}}"`,
      )

      if (!stdout.trim()) {
        this.logger.debug('No existing container found with exact name match')
        return null
      }

      const lines = stdout.trim().split('\n')
      for (const line of lines) {
        const [id, status] = line.split(',')
        if (id && status) {
          this.logger.debug('Found existing container', {
            containerId: id.trim(),
            status: status.trim(),
          })
          return { id: id.trim(), status: status.trim() }
        }
      }

      return null
    } catch (error) {
      this.logger.debug('Error checking for existing container', { error })
      return null
    }
  }

  private async removeExistingContainer(containerId: string): Promise<void> {
    try {
      this.logger.info('Removing existing container', { containerId })

      // First try to stop the container if it's running
      try {
        await execAsync(`docker stop ${containerId}`)
        this.logger.debug('Container stopped', { containerId })
      } catch (stopError) {
        // Container might already be stopped, continue with removal
        this.logger.debug('Container stop failed (may already be stopped)', { containerId })
      }

      // Remove the container
      await execAsync(`docker rm ${containerId}`)
      this.logger.info('Container removed successfully', { containerId })
    } catch (error) {
      this.logger.error('Failed to remove existing container', error as Error, { containerId })
      throw error
    }
  }

  private async createContainer(): Promise<void> {
    let createCommand = `docker create --name "${this.containerName}"`

    // Add moatless labels for easy cleanup
    createCommand += ` --label "created-by=moatless"`
    createCommand += ` --label "moatless.task-id=${this.taskId}"`
    createCommand += ` --label "moatless.created-at=${new Date().toISOString()}"`

    // Set working directory to /repo
    createCommand += ` -w "/repo"`

    // Add volume mappings
    const dataDir = process.env.DATA_DIR || './data'
    createCommand += ` -v "${this.workspacePath}:/repo"`
    createCommand += ` -v "${dataDir}:/data"`

    // Add main repo .git directory mount for worktrees
    if (this.repositoryConfig.useGitWorktrees) {
      // The repository path is the main repository path when using worktrees
      const mainRepoPath = this.repositoryConfig.path
      createCommand += ` -v "${mainRepoPath}/.git:/main-repo-git:ro"`
      createCommand += ` -e "MAIN_REPO_PATH=${mainRepoPath}"`
      createCommand += ` -e "USE_GIT_WORKTREES=true"`

      // Extract worktree name for git configuration
      const worktreeName = path.basename(this.workspacePath)
      createCommand += ` -e "GIT_WORKTREE_NAME=${worktreeName}"`
    }

    // Mount Claude config directory
    const containerClaudeConfigDir = '/home/ubuntu/.claude'
    const hostClaudeConfigDir =
      process.env.CLAUDE_CONFIG_DIR || path.join(process.env.HOME || '', '.claude')

    if (hostClaudeConfigDir && existsSync(hostClaudeConfigDir)) {
      createCommand += ` -v "${hostClaudeConfigDir}:${containerClaudeConfigDir}"`
    }

    if (!this.workerPortMapping) {
      throw new Error('Worker port not allocated')
    }

    // Add port mappings
    createCommand += ` -p ${this.workerPortMapping.hostPort}:${this.workerPortMapping.containerPort}`

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
    createCommand += ` -e "LOG_LEVEL=debug"`

    // Pass through Claude authentication environment variables
    if (process.env.CLAUDE_AUTH_DIR) {
      createCommand += ` -e "CLAUDE_AUTH_DIR=${process.env.CLAUDE_AUTH_DIR}"`
    }
    if (process.env.ANTHROPIC_API_KEY) {
      createCommand += ` -e "ANTHROPIC_API_KEY=${process.env.ANTHROPIC_API_KEY}"`
    }
    createCommand += ` -e "CLAUDE_CONFIG_DIR=${containerClaudeConfigDir}"`
    createCommand += ` -e "HOME=/repo"`

    if (process.env.CLAUDE_CODE_MOCK) {
      createCommand += ` -e "CLAUDE_CODE_MOCK=${process.env.CLAUDE_CODE_MOCK}"`
    }

    // Add repository-specific environment variables
    const envVars = this.repositoryConfig.codeExecution.envVars || {}
    for (const [key, value] of Object.entries(envVars)) {
      createCommand += ` -e "${key}=${value}"`
    }

    // Pass setup commands to container
    const setupCommands = this.repositoryConfig.codeExecution.setupCommands || []
    if (setupCommands.length > 0) {
      // Encode setup commands as JSON and pass as environment variable
      const setupCommandsJson = JSON.stringify(setupCommands)
      createCommand += ` -e "SETUP_COMMANDS=${setupCommandsJson.replace(/"/g, '\\"')}"`
    }

    createCommand += ` ${this.image}`

    this.logger.debug('Creating container', { createCommand })

    try {
      const { stdout } = await execAsync(createCommand)
      const containerId = stdout.trim()
      this.logger.info('Container created', { containerName: this.containerName, containerId })
    } catch (error) {
      this.logger.error('Failed to create container', error as Error)
      throw error
    }
  }

  private async startContainer(): Promise<void> {
    this.logger.info('Starting container', { containerName: this.containerName })

    try {
      // Check if container is already running
      const { stdout } = await execAsync(
        `docker ps --filter "name=^/${this.containerName}$" --format "{{.ID}}"`,
      )

      if (stdout.trim()) {
        this.logger.info('Container is already running', { containerName: this.containerName })
        return
      }

      // Container is not running, start it
      await execAsync(`docker start ${this.containerName}`)
      this.logger.info('Container started', { containerName: this.containerName })
    } catch (error) {
      this.logger.error('Failed to start container', error as Error)
      throw error
    }
  }

  /**
   * Ensures that the container is ready for command execution or agent creation.
   * If the container is not setup, it will attempt to set it up.
   * If setup is already in progress, it will wait for completion.
   */
  private async ensureContainerReady(): Promise<void> {
    const status = this.getStatus()

    switch (status) {
      case 'ready':
        // Container is already ready, nothing to do
        return

      case 'not_setup':
        // Container not setup, start setup process
        this.logger.info('Container not setup, starting setup process')
        await this.setup()
        // After setup, wait for container to be ready
        await this.waitForContainerReady()
        return

      case 'initializing':
        // Setup is in progress, wait for completion
        this.logger.info('Container setup in progress, waiting for completion')
        await this.waitForContainerReady()
        return

      case 'error': {
        // Container setup failed, throw error
        const error = this.getError() || 'Container setup failed'
        throw new Error(`Container is in error state: ${error}`)
      }

      default:
        throw new Error(`Unexpected container status: ${status}`)
    }
  }

  /**
   * Waits for the container to be ready for command execution or agent creation.
   * This relies on Docker's health check to determine readiness.
   */
  private async waitForContainerReady(): Promise<void> {
    const maxWaitTime = 180000 // 3 minute timeout for waiting
    const checkInterval = 2000 // 2 seconds (matches Docker health check interval)
    let elapsed = 0
    let lastHealthStatus = ''

    this.logger.debug('Waiting for container to become healthy', {
      containerName: this.containerName,
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
        this.logger.info('Container is healthy and ready', {
          elapsedMs: elapsed,
          containerName: this.containerName,
          workerPort: this.workerPortMapping.hostPort,
        })
        return
      }

      if (status === 'error') {
        // Get more details about the error
        try {
          const healthLogs = execSync(
            `docker inspect --format='{{range .State.Health.Log}}{{.Output}}{{end}}' "${this.containerName}"`,
            { encoding: 'utf8', timeout: 2000 },
          ).trim()
          this.logger.error('Container health check failed', new Error('Container unhealthy'), {
            healthLogs,
            containerName: this.containerName,
          })
        } catch {
          // Ignore errors getting health logs
        }
        const error = this.getError() || 'Container health check failed'
        throw new Error(`Container failed: ${error}`)
      }

      if (status === 'not_setup') {
        throw new Error('Container lost during wait - setup may have failed')
      }

      // Status is 'initializing', check health status for more details
      try {
        const healthStatus = execSync(
          `docker inspect --format='{{.State.Health.Status}}' "${this.containerName}"`,
          { encoding: 'utf8', timeout: 1000 },
        ).trim()

        if (healthStatus !== lastHealthStatus) {
          this.logger.debug('Container health status changed', {
            from: lastHealthStatus || 'none',
            to: healthStatus,
            elapsedMs: elapsed,
          })
          lastHealthStatus = healthStatus
        }
      } catch {
        // Ignore errors getting health status
      }

      // Log progress every 10 seconds
      if (elapsed % 10000 === 0 && elapsed > 0) {
        this.logger.debug('Container still starting, waiting for health check', {
          elapsedMs: elapsed,
          remainingMs: maxWaitTime - elapsed,
          containerName: this.containerName,
          lastHealthStatus,
        })
      }

      await new Promise((resolve) => setTimeout(resolve, checkInterval))
      elapsed += checkInterval
    }

    throw new Error(`Container did not become healthy within timeout (${maxWaitTime}ms)`)
  }

  async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
    // Ensure container is ready before executing command
    await this.ensureContainerReady()

    // Double-check we have container name and worker port mapping
    if (!this.containerName) {
      throw new Error('Container name not available')
    }
    if (!this.workerPortMapping) {
      throw new Error('Worker port mapping not available - container setup incomplete')
    }

    this.logger.debug(`Running command via docker exec: ${command}`)

    const startTime = Date.now()

    try {
      // Build docker exec command with git environment variables if needed
      let dockerCommand = 'docker exec -w /repo'

      // Add git environment variables if using worktrees
      if (this.repositoryConfig.useGitWorktrees) {
        const worktreeName = path.basename(this.workspacePath)
        dockerCommand += ` -e "GIT_DIR=/main-repo-git/worktrees/${worktreeName}"`
        dockerCommand += ` -e "GIT_WORK_TREE=/repo"`
      }

      dockerCommand += ` ${this.containerName} /bin/bash -c "${command.replace(/"/g, '\\"')}"`

      const { stdout, stderr } = await execAsync(dockerCommand, { timeout })

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
    await this.ensureContainerReady()

    this.logger.info('Starting server via worker')

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/start`,
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
      containerName: this.containerName,
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
    await this.ensureContainerReady()

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
    await this.ensureContainerReady()

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

  async stopAgent(): Promise<boolean> {
    if (!this.runningAgent) {
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
    this.logger.info('Tearing down DockerEnvironment', { force, containerName: this.containerName })

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

  setRunningAgent(agent: BaseAgent): void {
    this.runningAgent = agent
  }

  getRunningAgent(): BaseAgent | undefined {
    return this.runningAgent
  }

  private async cleanup(): Promise<void> {
    if (!this.containerName) {
      return
    }

    try {
      // Stop container
      await execAsync(`docker stop ${this.containerName}`)
      this.logger.info('Container stopped', { containerName: this.containerName })
    } catch (error) {
      this.logger.warn('Failed to stop container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    try {
      // Remove container
      await execAsync(`docker rm ${this.containerName}`)
      this.logger.info('Container removed', { containerName: this.containerName })
    } catch (error) {
      this.logger.warn('Failed to remove container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
}
