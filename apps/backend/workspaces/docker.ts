import type { GitChanges, GitStatus } from '@moatless/types/types/api'
import type { RepositoryConfig } from '@moatless/types/types/repository'
import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@moatless/types/types/workspace'
import { DockerEnvironment } from '../environments/docker-environment.js'
import type { BaseAgent, EventService } from '../services/index.js'
import type { GitDiffResponse } from '../utils/index.js'
import { createTaskLogger } from '../utils/index.js'
import { BaseWorkspace, type FileSearchRequest, type FileSearchResponse } from './base.js'
import type { LocalWorkspace } from './local.js'

export class DockerWorkspace extends BaseWorkspace {
  private environment: DockerEnvironment
  private localWorkspace: LocalWorkspace
  private logger

  get type(): 'docker' {
    return 'docker'
  }

  get status(): 'initializing' | 'ready' | 'error' | 'cleaning' {
    return this.environment.getStatus()
  }

  get createdAt(): Date {
    return this.environment.getCreatedAt()
  }

  get error(): string | undefined {
    return this.environment.getError()
  }

  getContainerId(): string {
    return this.environment.getContainerId()
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

    // Initialize environment with the local workspace path
    this.environment = new DockerEnvironment(
      taskId,
      repositoryConfig,
      eventService,
      localWorkspace.getWorkspacePath(),
    )
  }

  async setup(iterationNumber?: number): Promise<void> {
    try {
      await this.environment.setup(iterationNumber)
    } catch (error) {
      this.logger.error('DockerWorkspace setup failed', error as Error)
      throw error
    }
  }

  async teardown(force = false): Promise<void> {
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

    // Teardown the environment
    await this.environment.teardown(force)
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
    return this.environment.runCommand(command, timeout)
  }

  async triggerAgent(
    agentType: string,
    message: string,
    iterationNumber: number,
    resumeSessionId?: string,
    model?: string,
  ): Promise<BaseAgent> {
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
      })
      .catch((error: any) => {
        console.error('Agent execution failed in worker:', error)
      })

    // Return immediately (execution happens asynchronously in worker)
    return {} as BaseAgent
  }

  private async executeWorkerHttpRequest(endpoint: string, data: any): Promise<any> {
    // This would need to be implemented to send HTTP requests to the worker
    // For now, just log the request
    this.logger.debug('Would execute worker HTTP request', { endpoint, data })
    return Promise.resolve()
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
    return this.environment.isRunning()
  }

  getStatus(): WorkspaceStatus {
    const envStatus = this.environment.getStatus()
    switch (envStatus) {
      case 'ready':
        return WorkspaceStatus.IDLE
      case 'initializing':
        return WorkspaceStatus.SETTING_UP
      case 'error':
        return WorkspaceStatus.IDLE // Map error to idle for now
      case 'cleaning':
        return WorkspaceStatus.IDLE // Map cleaning to idle for now
      default:
        return WorkspaceStatus.NOT_SETUP
    }
  }

  async hasUncommittedChanges(): Promise<boolean> {
    // Delegate to local workspace for consistent git operations
    return this.localWorkspace.hasUncommittedChanges()
  }

  async mergeChanges(): Promise<string> {
    throw new Error('Merge not supported for Docker workspaces')
  }

  async stopAgent(): Promise<boolean> {
    return this.environment.stopAgent()
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

  // Preview server methods - delegate to environment
  async startServer(): Promise<ServerStatus> {
    return this.environment.startServer!()
  }

  async stopServer(): Promise<boolean> {
    return this.environment.stopServer!()
  }

  async getServerStatus(): Promise<ServerStatus> {
    return this.environment.getServerStatus!()
  }

  async getServerLogs(lines = 100): Promise<string> {
    return this.environment.getServerLogs!(lines)
  }

  toWorkspaceInfo(): Workspace {
    return {
      id: this.taskId,
      type: 'docker' as const,
      repositoryId: this.repositoryConfig.id,
      status: this.status,
      createdAt: this.createdAt,
      containerId: this.getContainerId(),
      error: this.error,
    }
  }

  getWorkspacePath(): string {
    // Delegate to local workspace to get the actual workspace path
    return this.localWorkspace.getWorkspacePath()
  }

  setRunningAgent(agent: BaseAgent): void {
    this.environment.setRunningAgent(agent)
  }

  async isWorkspacePathValid(): Promise<boolean> {
    // Delegate to local workspace for path validation
    return this.localWorkspace.isWorkspacePathValid()
  }
}
