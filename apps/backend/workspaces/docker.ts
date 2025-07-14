import { exec } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { GitChanges, GitStatus } from '@moatless/types/types/api'
import { EventType } from '@moatless/types/types/event'
import type { RepositoryConfig } from '@moatless/types/types/repository'
import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@moatless/types/types/workspace'
import type { BaseAgent, EventService } from '../services/index.js'
import type { GitDiffResponse } from '../utils/index.js'
import { createTaskLogger } from '../utils/index.js'
import { BaseWorkspace, type FileSearchRequest, type FileSearchResponse } from './base.js'
import type { LocalWorkspace } from './local.js'

const execAsync = promisify(exec)

export class DockerWorkspace extends BaseWorkspace {
  private containerId = ''
  private containerName = ''
  private image = ''
  private setupInProgress = false
  private setupComplete = false
  private runningAgent?: BaseAgent
  private _createdAt = new Date()
  private _error?: string
  private localWorkspace: LocalWorkspace
  private previewPortMapping?: { containerPort: number; hostPort: number }
  private workerPort = 8001
  private logger

  get type(): 'docker' {
    return 'docker'
  }

  get status(): 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) return 'error'
    if (this.setupInProgress) return 'initializing'
    if (this.setupComplete) return 'ready'
    return 'initializing'
  }

  get createdAt(): Date {
    return this._createdAt
  }

  get error(): string | undefined {
    return this._error
  }

  getContainerId(): string {
    return this.containerId
  }

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    eventService: EventService,
    localWorkspace: LocalWorkspace,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.localWorkspace = localWorkspace
    this.logger = createTaskLogger(taskId)
  }

  async setup(iterationNumber?: number): Promise<void> {
    this.setupInProgress = true
    try {
      // Emit setup started event

      this.image =
        this.repositoryConfig.codeExecution.dockerImage || 'moatless-vibe-universal:latest'

      // Create container
      this.containerName = this.repositoryConfig.codeExecution.containerName
        ? `${this.repositoryConfig.codeExecution.containerName}-${this.taskId}`
        : `moatless-task-${this.taskId}`

      // Docker workspaces always run backend inside container
      const actualImage = this.image

      let createCommand = `docker create --name "${this.containerName}"`

      // Add moatless labels for easy cleanup
      createCommand += ` --label "created-by=moatless"`
      createCommand += ` --label "moatless.task-id=${this.taskId}"`
      createCommand += ` --label "moatless.workspace-type=docker"`
      createCommand += ` --label "moatless.created-at=${new Date().toISOString()}"`

      // Set working directory to /repo
      createCommand += ` -w "/repo"`

      // Add volume mappings - mount both workspace and data directories
      const workspacePath = this.localWorkspace.getWorkspacePath()
      const dataDir = process.env.DATA_DIR || './data'

      // Mount workspace to /repo
      createCommand += ` -v "${workspacePath}:/repo"`

      // Mount data directory for types storage
      createCommand += ` -v "${dataDir}:/data"`

      // Define container paths for Claude configuration
      const containerClaudeConfigDir = '/home/node/.claude'

      // Mount Claude config directory to container user's home
      const hostClaudeConfigDir =
        process.env.CLAUDE_CONFIG_DIR || path.join(process.env.HOME || '', '.claude')

      if (hostClaudeConfigDir && existsSync(hostClaudeConfigDir)) {
        createCommand += ` -v "${hostClaudeConfigDir}:${containerClaudeConfigDir}"`
      }

      // Add environment variables for worker mode
      // Use host.docker.internal to reach the host from within the container
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
      createCommand += ` -e "PORT=8001"` // Worker runs on different port
      createCommand += ` -e "LOG_LEVEL=debug"`

      // Pass through Claude authentication environment variables
      if (process.env.CLAUDE_AUTH_DIR) {
        createCommand += ` -e "CLAUDE_AUTH_DIR=${process.env.CLAUDE_AUTH_DIR}"`
      }
      if (process.env.ANTHROPIC_API_KEY) {
        createCommand += ` -e "ANTHROPIC_API_KEY=${process.env.ANTHROPIC_API_KEY}"`
      }
      // Set Claude config directory path to container location
      createCommand += ` -e "CLAUDE_CONFIG_DIR=${containerClaudeConfigDir}"`
      // Set HOME environment variable for container user (node user's home is /home/node)
      createCommand += ` -e "HOME=/home/node"`
      // Pass mock mode for tests
      if (process.env.CLAUDE_CODE_MOCK) {
        createCommand += ` -e "CLAUDE_CODE_MOCK=${process.env.CLAUDE_CODE_MOCK}"`
      }

      if (this.repositoryConfig.codeExecution.envVars) {
        for (const [key, value] of Object.entries(this.repositoryConfig.codeExecution.envVars)) {
          createCommand += ` -e "${key}=${value}"`
        }
      }

      // Add resource limits if specified
      if (this.repositoryConfig.codeExecution.resources) {
        const { cpuLimit, memoryLimit } = this.repositoryConfig.codeExecution.resources
        if (cpuLimit) {
          createCommand += ` --cpus="${cpuLimit}"`
        }
        if (memoryLimit) {
          createCommand += ` --memory="${memoryLimit}"`
        }
      }

      // Start the backend directly
      // Working directory is set to /repo where the workspace is mounted
      createCommand += ` "${actualImage}"`

      const { stdout } = await execAsync(createCommand, { timeout: 30000 })
      this.containerId = stdout.trim()

      // Start container
      await execAsync(`docker start "${this.containerId}"`, { timeout: 10000 })

      // Wait for worker to be ready
      await this.waitForWorkerReady()

      // Run setup commands if specified
      if (this.repositoryConfig.codeExecution.setupCommands) {
        const commands = this.repositoryConfig.codeExecution.setupCommands.split('\n')
        for (const command of commands) {
          const trimmedCommand = command.trim()
          if (!trimmedCommand) continue

          try {
            const result = await this.runCommand(trimmedCommand)

            if (result.exitCode === 0) {
              await this.eventService.emitEvent({
                eventId: randomUUID(),
                agentType: 'system',
                type: EventType.WORKSPACE,
                subType: 'command_completed',
                taskId: this.taskId,
                iterationId: iterationNumber,
                timestamp: new Date(),
                isError: false,
                payload: {
                  command: trimmedCommand,
                  durationMs: result.durationMs,
                },
              })
            } else {
              await this.eventService.emitEvent({
                eventId: randomUUID(),
                agentType: 'system',
                type: EventType.WORKSPACE,
                subType: 'command_failed',
                taskId: this.taskId,
                iterationId: iterationNumber,
                timestamp: new Date(),
                isError: true,
                payload: {
                  command: trimmedCommand,
                  error: result.output,
                  exitCode: result.exitCode,
                },
              })
            }
          } catch (error) {
            await this.eventService.emitEvent({
              eventId: randomUUID(),
              agentType: 'system',
              type: EventType.WORKSPACE,
              subType: 'command_failed',
              taskId: this.taskId,
              iterationId: iterationNumber,
              timestamp: new Date(),
              isError: true,
              payload: {
                command: trimmedCommand,
                error: error instanceof Error ? error.message : String(error),
              },
            })
          }
        }
      }

      // Emit setup completed event
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        agentType: 'system',
        type: EventType.WORKSPACE,
        subType: 'setup_completed',
        taskId: this.taskId,
        iterationId: iterationNumber,
        timestamp: new Date(),
        isError: false,
        payload: {
          containerId: this.containerId,
        },
      })

      this.setupComplete = true
    } catch (error) {
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        agentType: 'system',
        type: EventType.WORKSPACE,
        subType: 'setup_failed',
        taskId: this.taskId,
        iterationId: iterationNumber,
        timestamp: new Date(),
        isError: true,
        payload: {
          error: error instanceof Error ? error.message : String(error),
        },
      })
      this._error = error instanceof Error ? error.message : String(error)
      throw error
    } finally {
      this.setupInProgress = false
    }
  }

  async teardown(force = false): Promise<void> {
    if (!this.containerId) return

    // Check for uncommitted changes first
    if (!force) {
      try {
        const hasChanges = await this.hasUncommittedChanges()
        if (hasChanges) {
          throw new Error('Workspace has uncommitted changes. Use force=true to close anyway.')
        }
      } catch (error) {
        // If we can't check for changes during graceful shutdown, just log and continue
        console.warn('Could not check for uncommitted changes during teardown:', error)
      }
    }

    // Stop the running agent if any
    if (this.runningAgent) {
      try {
        await this.executeWorkerHttpRequest('/stop-agent', { taskId: this.taskId })
      } catch (error) {
        console.error('Error stopping agent:', error)
      }
      this.runningAgent = undefined
    }

    try {
      // Stop container with shorter timeout for graceful shutdown
      const stopTimeout = force ? 5000 : 10000
      await execAsync(`docker stop "${this.containerId}"`, { timeout: stopTimeout })

      // Remove container
      await execAsync(`docker rm "${this.containerId}"`, { timeout: 5000 })
    } catch (error) {
      if (force) {
        // During graceful shutdown, log errors but don't throw
        console.error(`Error cleaning up Docker workspace ${this.taskId}:`, error)
      } else {
        throw new Error(
          `Failed to cleanup Docker workspace: ${error instanceof Error ? error.message : String(error)}`,
        )
      }
    }

    this.setupComplete = false
    this.containerId = ''
  }

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    // Delegate to local workspace for consistent file search behavior
    return this.localWorkspace.searchFiles(request)
  }

  async getGitDiff(): Promise<GitDiffResponse> {
    // Delegate to local workspace for consistent git operations
    return this.localWorkspace.getGitDiff()
  }

  async getGitStatus(): Promise<GitStatus> {
    // Delegate to local workspace for consistent git operations
    return this.localWorkspace.getGitStatus()
  }

  async getGitChanges(): Promise<GitChanges> {
    // Delegate to local workspace for consistent git operations
    return this.localWorkspace.getGitChanges()
  }

  async runCommand(command: string, timeout?: number): Promise<CommandResult> {
    const startTime = Date.now()

    try {
      const dockerCommand = `docker exec "${this.containerId}" ${command}`
      const { stdout, stderr } = await execAsync(dockerCommand, {
        timeout: timeout || 120000, // Default 2 minutes
      })

      // Combine stdout and stderr
      let output = stdout
      if (stderr) {
        output += output ? `\n${stderr}` : stderr
      }

      return {
        command,
        output,
        exitCode: 0,
        durationMs: Date.now() - startTime,
      }
    } catch (error: unknown) {
      const errorObj = error as {
        stdout?: string
        stderr?: string
        message?: string
        code?: number
      }
      const output = errorObj.stdout || ''
      const stderr = errorObj.stderr || ''
      const combinedOutput = output + (stderr ? `\n${stderr}` : '')

      return {
        command,
        output: combinedOutput || errorObj.message || 'Unknown error',
        exitCode: errorObj.code || -1,
        durationMs: Date.now() - startTime,
      }
    }
  }

  async triggerAgent(
    agentType: string,
    message: string,
    iterationNumber: number,
    resumeSessionId?: string,
    model?: string,
  ): Promise<BaseAgent> {
    // Mark agent as running
    this.runningAgent = {} as BaseAgent

    // Execute agent asynchronously in worker container via HTTP
    this.executeWorkerHttpRequest('/execute-agent', {
      taskId: this.taskId,
      message,
      iteration: {
        iterationNumber,
        startTime: new Date(),
        status: 'active' as const,
        costUsd: 0,
        durationMs: 0,
        messageCount: 1,
      },
      agentType,
      resumeSessionId,
      model,
    })
      .then(() => {
        console.log('Agent execution completed in worker')
        this.runningAgent = undefined
      })
      .catch((error) => {
        console.error('Agent execution failed in worker:', error)
        this.runningAgent = undefined
      })

    // Return immediately (execution happens asynchronously in worker)
    return {} as BaseAgent
  }

  async start(agentType: string, message: string, iterationNumber: number): Promise<void> {
    if (this.getStatus() === WorkspaceStatus.RUNNING) {
      throw new Error(`Workspace ${this.taskId} is already running`)
    }

    // Run setup
    await this.setup(iterationNumber)

    // Trigger the agent
    await this.triggerAgent(agentType, message, iterationNumber)
  }

  isRunning(): boolean {
    return this.runningAgent !== undefined
  }

  getStatus(): WorkspaceStatus {
    // Check if agent is running
    if (this.runningAgent) {
      return WorkspaceStatus.RUNNING
    }

    // Check if setup is in progress
    if (this.setupInProgress) {
      return WorkspaceStatus.SETTING_UP
    }

    // Check if container exists
    if (this.containerId && this.setupComplete) {
      return WorkspaceStatus.IDLE
    }

    // Otherwise, workspace is not set up
    return WorkspaceStatus.NOT_SETUP
  }

  async hasUncommittedChanges(): Promise<boolean> {
    // Delegate to local workspace for consistent git operations
    return this.localWorkspace.hasUncommittedChanges()
  }

  async mergeChanges(): Promise<string> {
    throw new Error('Merge not supported for Docker workspaces')
  }

  async stopAgent(): Promise<boolean> {
    if (!this.runningAgent) {
      console.log(`No agent running for task ${this.taskId}`)
      return false
    }

    console.log(`Stopping agent for task ${this.taskId}`)
    try {
      // Stop agent via worker HTTP endpoint
      await this.executeWorkerHttpRequest('/stop-agent', { taskId: this.taskId })
      this.runningAgent = undefined
      console.log(`Agent stopped successfully for task ${this.taskId}`)
      return true
    } catch (error) {
      console.error(`Error stopping agent for task ${this.taskId}:`, error)
      this.runningAgent = undefined
      return false
    }
  }

  async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
    // Delegate to local workspace for consistent file operations
    return this.localWorkspace.listFiles(relativePath, recursive)
  }

  async readFile(filePath: string): Promise<string> {
    // Delegate to local workspace for consistent file operations
    return this.localWorkspace.readFile(filePath)
  }

  async writeFile(filePath: string, content: string): Promise<void> {
    // Delegate to local workspace for consistent file operations
    return this.localWorkspace.writeFile(filePath, content)
  }

  // Preview server methods - delegate to worker
  async startServer(): Promise<ServerStatus> {
    this.logger.info('Starting preview server in Docker container')

    try {
      const response = await this.makeWorkerRequest('/preview/start', 'POST')
      const containerStatus = response.data

      // Map container response to host-accessible URL
      this.previewPortMapping = {
        containerPort: containerStatus.port,
        hostPort: containerStatus.port, // In Docker setup, we map container port to same host port
      }

      return {
        running: containerStatus.running,
        configured: containerStatus.configured,
        url: containerStatus.url?.replace('localhost', 'localhost'), // Keep as localhost for host access
        uptimeSeconds: containerStatus.uptimeSeconds,
      }
    } catch (error) {
      this.logger.error('Failed to start preview server', error)
      throw error
    }
  }

  async stopServer(): Promise<boolean> {
    this.logger.info('Stopping preview server in Docker container')

    try {
      const response = await this.makeWorkerRequest('/preview/stop', 'POST')
      const success = response.success

      if (success) {
        this.previewPortMapping = undefined
      }

      return success
    } catch (error) {
      this.logger.error('Failed to stop preview server', error)
      return false
    }
  }

  async getServerStatus(): Promise<ServerStatus> {
    // For Docker workspaces, we need to query the worker for current status
    // This is an async method that can query the worker
    try {
      const response = await this.makeWorkerRequest('/preview/status', 'GET')
      const containerStatus = response.data

      return {
        running: containerStatus.running,
        configured: containerStatus.configured,
        url: containerStatus.url?.replace('localhost', 'localhost'), // Keep as localhost for host access
        uptimeSeconds: containerStatus.uptimeSeconds,
      }
    } catch (_error) {
      // If we can't reach the worker, return basic status based on local state
      return {
        running: this.previewPortMapping !== undefined,
        configured: true,
        url: this.previewPortMapping
          ? `http://localhost:${this.previewPortMapping.hostPort}`
          : undefined,
        uptimeSeconds: 0,
      }
    }
  }

  async getServerLogs(lines = 100): Promise<string> {
    this.logger.debug('Fetching preview server logs from Docker container', { lines })

    try {
      const response = await this.makeWorkerRequest('/preview/logs', 'GET', { lines })
      return response.logs || 'No logs available'
    } catch (error) {
      this.logger.error('Failed to fetch preview server logs', error)
      return `Failed to fetch logs: ${error instanceof Error ? error.message : String(error)}`
    }
  }

  toWorkspaceInfo(): Workspace {
    return {
      id: this.taskId,
      type: 'docker' as const,
      repositoryId: this.repositoryConfig.id,
      status: this.status,
      createdAt: this.createdAt,
      containerId: this.containerId,
      error: this.error,
    }
  }

  getWorkspacePath(): string {
    // Delegate to local workspace to get the actual workspace path
    return this.localWorkspace.getWorkspacePath()
  }

  async isWorkspacePathValid(): Promise<boolean> {
    // Check container status
    if (!this.containerId) return false

    try {
      // Check if container exists and is running
      const { stdout } = await execAsync(
        `docker ps -a --filter "id=${this.containerId}" --format "{{.Status}}"`,
      )
      const status = stdout.trim()

      // Container exists if we get any status back
      if (!status) return false

      // Check if container is running (status starts with "Up")
      return status.startsWith('Up')
    } catch {
      return false
    }
  }

  /**
   * Ensure container is running, restart if needed
   */
  async ensureContainerRunning(): Promise<void> {
    if (!this.containerId) {
      throw new Error('No container ID available, need to run setup first')
    }

    try {
      // Check if container exists and its status
      const { stdout } = await execAsync(
        `docker ps -a --filter "id=${this.containerId}" --format "{{.Status}}"`,
      )
      const status = stdout.trim()

      if (!status) {
        // Container doesn't exist, need to recreate
        this.containerId = ''
        this.setupComplete = false
        throw new Error('Container no longer exists, need to run setup again')
      }

      if (status.startsWith('Up')) {
        // Container is already running
        return
      }

      // Container exists but is stopped, restart it
      await execAsync(`docker start "${this.containerId}"`, { timeout: 10000 })

      // Wait for worker to be ready
      await this.waitForWorkerReady()
    } catch (error) {
      // If restart fails, mark as needing setup
      this.setupComplete = false
      throw error
    }
  }

  /**
   * Wait for worker backend to be ready
   */
  private async waitForWorkerReady(): Promise<void> {
    const maxAttempts = 30
    const delayMs = 1000

    for (let i = 0; i < maxAttempts; i++) {
      try {
        // Check health endpoint of worker running in container
        const response = await execAsync(
          `docker exec "${this.containerId}" curl -s http://localhost:8001/health`,
          { timeout: 5000 },
        )
        console.log(response)

        const health = JSON.parse(response.stdout)
        if (health.status === 'ok' && health.mode === 'worker') {
          console.log('Worker backend is ready')
          return
        }
      } catch (error) {
        console.log(error)
      }

      await new Promise((resolve) => setTimeout(resolve, delayMs))
    }

    throw new Error('Worker backend failed to start within 30 seconds')
  }

  /**
   * Make HTTP request to worker backend running in container
   */
  private async makeWorkerRequest(
    path: string,
    method: 'GET' | 'POST' = 'POST',
    payload?: any,
  ): Promise<any> {
    let curlCommand: string[]

    if (method === 'GET') {
      curlCommand = [
        'curl',
        '-X',
        'GET',
        `http://localhost:${this.workerPort}${path}`,
        '--max-time',
        '30',
      ]
    } else {
      const requestPayload = JSON.stringify(payload || {})
      // Escape JSON for shell command
      const escapedPayload = requestPayload.replace(/'/g, "'\\''")

      curlCommand = [
        'curl',
        '-X',
        method,
        `http://localhost:${this.workerPort}${path}`,
        '-H',
        '"Content-Type: application/json"',
        '-d',
        `'${escapedPayload}'`,
        '--max-time',
        '30',
      ]
    }

    const { stdout, stderr } = await execAsync(
      `docker exec "${this.containerId}" sh -c "${curlCommand.join(' ')}"`,
      { timeout: 35000 },
    )

    // Parse and return response
    try {
      const response = JSON.parse(stdout)
      if (!response.success && method !== 'GET') {
        throw new Error(`Worker request failed: ${response.error || 'Unknown error'}`)
      }
      return response
    } catch (_parseError) {
      throw new Error(`Worker request failed: ${stderr || stdout || 'Invalid response'}`)
    }
  }

  /**
   * Execute HTTP request to worker backend running in container
   */
  private async executeWorkerHttpRequest(path: string, payload: any): Promise<void> {
    const requestPayload = JSON.stringify(payload)

    // Escape JSON for shell command
    const escapedPayload = requestPayload.replace(/'/g, "'\\''")

    const curlCommand = [
      'curl',
      '-X',
      'POST',
      `http://localhost:8001${path}`,
      '-H',
      '"Content-Type: application/json"',
      '-d',
      `'${escapedPayload}'`,
      '--max-time',
      '30',
    ].join(' ')

    const { stdout, stderr } = await execAsync(
      `docker exec "${this.containerId}" sh -c "${curlCommand}"`,
      { timeout: 35000 },
    )

    // Check if response indicates success
    try {
      const response = JSON.parse(stdout)
      if (!response.success) {
        throw new Error(`Worker request failed: ${response.error || 'Unknown error'}`)
      }
    } catch (_parseError) {
      throw new Error(`Worker request failed: ${stderr || stdout || 'Invalid response'}`)
    }
  }
}
