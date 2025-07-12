import { exec, spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import { EventType } from '@moatless/shared/types/event'
import type { RepositoryConfig } from '@moatless/shared/types/repository'
import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@moatless/shared/types/workspace'
import type { BaseAgent } from '../services/agents/base-agent.js'
import type { EventService } from '../services/event.service.js'
import type { GitDiffResponse } from '../utils/git-utils.js'
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
  private eventStreamController?: AbortController

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

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    eventService: EventService,
    localWorkspace: LocalWorkspace,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.localWorkspace = localWorkspace
  }

  async setup(iterationNumber?: number): Promise<void> {
    this.setupInProgress = true
    try {
      // Emit setup started event

      this.image = this.repositoryConfig.codeExecution.dockerImage || 'moatless-worker:latest'

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

      // Mount data directory for shared storage
      createCommand += ` -v "${dataDir}:/data"`

      // Mount Claude config directory
      const claudeConfigDir =
        process.env.CLAUDE_CONFIG_DIR || path.join(process.env.HOME || '', '.claude')
      if (claudeConfigDir) {
        createCommand += ` -v "${claudeConfigDir}:${claudeConfigDir}"`
      }

      // Mount .claude.json file as .config.json in the Claude config directory
      const homeDir = process.env.HOME
      if (homeDir && claudeConfigDir) {
        const claudeJsonPath = path.join(homeDir, '.claude.json')
        const configJsonPath = path.join(claudeConfigDir, '.config.json')
        if (existsSync(claudeJsonPath)) {
          createCommand += ` -v "${claudeJsonPath}:${configJsonPath}:ro"`
        }
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

      // Pass through Claude authentication environment variables
      if (process.env.CLAUDE_AUTH_DIR) {
        createCommand += ` -e "CLAUDE_AUTH_DIR=${process.env.CLAUDE_AUTH_DIR}"`
      }
      if (process.env.ANTHROPIC_API_KEY) {
        createCommand += ` -e "ANTHROPIC_API_KEY=${process.env.ANTHROPIC_API_KEY}"`
      }
      // Pass through Claude config directory path
      if (claudeConfigDir) {
        createCommand += ` -e "CLAUDE_CONFIG_DIR=${claudeConfigDir}"`
      }
      // Set HOME environment variable to match host
      if (process.env.HOME) {
        createCommand += ` -e "HOME=${process.env.HOME}"`
      }
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

      console.log(createCommand)

      const { stdout } = await execAsync(createCommand, { timeout: 30000 })
      this.containerId = stdout.trim()
      console.log('containerId', stdout)

      // Start container
      console.log(`docker start "${this.containerId}"`)
      const startResult = await execAsync(`docker start "${this.containerId}"`, { timeout: 10000 })
      console.log('startResult', startResult)

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
      const hasChanges = await this.hasUncommittedChanges()
      if (hasChanges) {
        throw new Error('Workspace has uncommitted changes. Use force=true to close anyway.')
      }
    }

    // Stop event stream if running
    if (this.eventStreamController) {
      this.eventStreamController.abort()
      this.eventStreamController = undefined
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
      // Stop container
      await execAsync(`docker stop "${this.containerId}"`, { timeout: 10000 })

      // Remove container
      await execAsync(`docker rm "${this.containerId}"`, { timeout: 10000 })
    } catch (error) {
      throw new Error(
        `Failed to cleanup Docker workspace: ${error instanceof Error ? error.message : String(error)}`,
      )
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

    // Start event stream before triggering agent
    this.startEventStream()

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
        // Event stream will be stopped when agent stops
      })
      .catch((error) => {
        console.error('Agent execution failed in worker:', error)
        this.runningAgent = undefined
        if (this.eventStreamController) {
          this.eventStreamController.abort()
          this.eventStreamController = undefined
        }
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
      // Stop event stream first
      if (this.eventStreamController) {
        this.eventStreamController.abort()
        this.eventStreamController = undefined
      }

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

  // Server methods (TODO: implement these)
  async startServer(): Promise<never> {
    throw new Error('Server not yet implemented for Docker workspaces')
  }

  async stopServer(): Promise<boolean> {
    return false
  }

  getServerStatus(): ServerStatus {
    return {
      running: false,
      configured: false,
      uptimeSeconds: 0,
    }
  }

  async getServerLogs(_lines = 100): Promise<string> {
    return 'No server configured or initialized'
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
   * Wait for worker backend to be ready
   */
  private async waitForWorkerReady(): Promise<void> {
    const maxAttempts = 300
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
    } catch (parseError) {
      throw new Error(`Worker request failed: ${stderr || stdout || 'Invalid response'}`)
    }
  }

  /**
   * Start SSE event stream from worker
   */
  private startEventStream(): void {
    if (this.eventStreamController) {
      return // Already streaming
    }

    console.log(`Starting SSE event stream for task ${this.taskId}`)

    // Create abort controller for cleanup
    this.eventStreamController = new AbortController()

    // Connect to worker's event stream endpoint
    this.connectToWorkerEventStream()
  }

  /**
   * Connect to worker's SSE event stream and forward events
   */
  private async connectToWorkerEventStream(): Promise<void> {
    try {
      // Use spawn to stream output from docker exec
      const dockerExec = spawn('docker', [
        'exec',
        this.containerId,
        'curl',
        '-N', // No buffering for streaming
        '-H',
        'Accept: text/event-stream',
        'http://localhost:8001/events/stream',
      ])

      let buffer = ''

      // Handle streaming data
      dockerExec.stdout.on('data', async (data) => {
        buffer += data.toString()

        // Process complete lines
        const lines = buffer.split('\n')
        buffer = lines.pop() || '' // Keep incomplete line in buffer

        for (const line of lines) {
          if (line.trim()) {
            try {
              // Parse NDJSON event
              const event = JSON.parse(line)

              // Skip ping events
              if (event.type === 'ping') {
                continue
              }

              // Forward event to main event service
              await this.eventService.emitEvent(event)
            } catch (error) {
              console.error('Failed to parse event from worker:', error, line)
            }
          }
        }
      })

      // Handle errors
      dockerExec.stderr.on('data', (data) => {
        console.error('Worker event stream stderr:', data.toString())
      })

      // Handle process exit
      dockerExec.on('close', (code) => {
        console.log(`Worker event stream process exited with code ${code}`)
      })

      // Handle abort
      this.eventStreamController?.signal.addEventListener('abort', () => {
        dockerExec.kill()
      })
    } catch (error: any) {
      if (error.name === 'AbortError') {
        console.log('Worker event stream aborted')
      } else {
        console.error('Worker event stream error:', error)
      }
    }
  }
}
