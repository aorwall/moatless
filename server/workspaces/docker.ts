import { exec } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { promisify } from 'node:util'
import { EventType } from '@shared/types/event.js'
import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@shared/types/workspace.js'
import type { BaseAgent } from '../services/agents/base-agent.js'
import { BaseWorkspace, type FileSearchRequest, type FileSearchResponse } from './base.js'

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

  async setup(iterationNumber?: number): Promise<void> {
    this.setupInProgress = true
    try {
      // Emit setup started event

      this.image = this.repositoryConfig.codeExecution.image || 'node:18-alpine'
      const workdir = '/workspace'

      // Create container
      this.containerName = `moatless-task-${this.taskId}`

      let createCommand = `docker create --name "${this.containerName}" -w "${workdir}"`

      // Add volume mounts if specified
      if (this.repositoryConfig.codeExecution.volumes) {
        for (const volume of this.repositoryConfig.codeExecution.volumes) {
          createCommand += ` -v "${volume}"`
        }
      }

      // Add environment variables
      if (this.repositoryConfig.codeExecution.envVars) {
        for (const [key, value] of Object.entries(this.repositoryConfig.codeExecution.envVars)) {
          createCommand += ` -e "${key}=${value}"`
        }
      }

      createCommand += ` "${this.image}" sleep infinity`

      const { stdout } = await execAsync(createCommand, { timeout: 30000 })
      this.containerId = stdout.trim()

      // Start container
      await execAsync(`docker start "${this.containerId}"`, { timeout: 10000 })

      // For local repositories, we need to copy the files into the container
      if (this.repositoryConfig.repository.type === 'local') {
        // Copy repository files to container
        const copyCommand = `docker cp "${this.repositoryConfig.repository.path}/." "${this.containerId}:/workspace/"`
        await execAsync(copyCommand, { timeout: 60000 })
      }

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

    // Stop the running agent if any
    if (this.runningAgent) {
      try {
        await this.runningAgent.stop()
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

  async searchFiles(_request: FileSearchRequest): Promise<FileSearchResponse> {
    // TODO: Implement Docker file search
    throw new Error('Docker file search not yet implemented')
  }

  async getGitDiff(): Promise<string> {
    // TODO: Implement Docker git diff
    throw new Error('Git diff not supported for Docker workspaces yet')
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
    _agentType: string,
    _message: string,
    _iterationNumber: number,
  ): Promise<BaseAgent> {
    // TODO: Implement agent triggering for Docker workspaces
    throw new Error('Agent triggering not yet implemented for Docker workspaces')
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
    // TODO: Implement for Docker
    return false
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
      await this.runningAgent.stop()
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
    try {
      const command = recursive
        ? `docker exec "${this.containerId}" find "${relativePath}" -type f`
        : `docker exec "${this.containerId}" ls -1 "${relativePath}"`

      const { stdout } = await execAsync(command, { timeout: 10000 })
      return stdout
        .trim()
        .split('\n')
        .filter((line) => line.trim())
    } catch (error) {
      throw new Error(
        `Failed to list Docker files: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  async readFile(filePath: string): Promise<string> {
    try {
      const command = `docker exec "${this.containerId}" cat "${filePath}"`
      const { stdout } = await execAsync(command, { timeout: 10000 })
      return stdout
    } catch (error) {
      throw new Error(
        `Failed to read Docker file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  async writeFile(filePath: string, content: string): Promise<void> {
    try {
      // Create directory if needed
      const dirPath = filePath.substring(0, filePath.lastIndexOf('/'))
      if (dirPath && dirPath !== '.') {
        await execAsync(`docker exec "${this.containerId}" mkdir -p "${dirPath}"`, {
          timeout: 5000,
        })
      }

      // Write file using docker exec with tee
      const command = `docker exec -i "${this.containerId}" tee "${filePath}"`
      const child = exec(command, { timeout: 10000 })

      child.stdin?.write(content)
      child.stdin?.end()

      await new Promise<void>((resolve, reject) => {
        child.on('close', (code) => {
          if (code === 0) resolve()
          else reject(new Error(`Command failed with code ${code}`))
        })
        child.on('error', reject)
      })
    } catch (error) {
      throw new Error(
        `Failed to write Docker file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
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
    return this.containerId
  }

  async isWorkspacePathValid(): Promise<boolean> {
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
}
