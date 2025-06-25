import type { Environment } from '@shared/types/environment.js'
import type { CommandResult, Workspace, WorkspaceStatus } from '@shared/types/workspace.js'
import type { BaseAgent } from '../services/agents/base-agent.js'
import type { EventService } from '../services/event.service.js'

export interface FileSearchRequest {
  pattern?: string
  path?: string
  recursive?: boolean
  includeHidden?: boolean
}

export interface FileSearchResponse {
  files: string[]
  totalCount: number
}

export abstract class BaseWorkspace {
  protected taskId: string
  protected environment: Environment
  protected eventService: EventService

  // Properties that match the Workspace interface
  get id(): string {
    return this.taskId
  }

  abstract get type(): 'local' | 'docker'
  abstract get status(): 'initializing' | 'ready' | 'error' | 'cleaning'
  abstract get createdAt(): Date
  abstract get error(): string | undefined

  get environmentId(): string {
    return this.environment.id
  }

  constructor(taskId: string, environment: Environment, eventService: EventService) {
    this.taskId = taskId
    this.environment = environment
    this.eventService = eventService
  }

  /**
   * Set up the workspace (create worktree, container, etc.)
   */
  abstract setup(iterationNumber?: number): Promise<void>

  /**
   * Clean up workspace resources
   */
  abstract teardown(force?: boolean): Promise<void>

  /**
   * Search for files in the workspace
   */
  abstract searchFiles(request: FileSearchRequest): Promise<FileSearchResponse>

  /**
   * Get git diff for the workspace
   */
  abstract getGitDiff(): Promise<string>

  /**
   * Run a command in the workspace
   */
  abstract runCommand(command: string, timeout?: number): Promise<CommandResult>

  /**
   * Trigger an agent within this workspace
   */
  abstract triggerAgent(
    agentType: string,
    message: string,
    iterationNumber: number,
  ): Promise<BaseAgent>

  /**
   * Start the workspace by running setup and triggering agent
   */
  abstract start(agentType: string, message: string, iterationNumber: number): Promise<void>

  /**
   * Check if an agent is currently running in this workspace
   */
  abstract isRunning(): boolean

  /**
   * Get the current status of the workspace
   */
  abstract getStatus(): WorkspaceStatus

  /**
   * Check if the workspace has uncommitted git changes
   */
  abstract hasUncommittedChanges(): Promise<boolean>

  /**
   * Merge changes from workspace branch back to the original branch
   */
  abstract mergeChanges(): Promise<string>

  /**
   * Stop the running agent for this workspace
   */
  abstract stopAgent(): Promise<boolean>

  /**
   * Start the server if configured
   */
  abstract startServer?(): Promise<any>

  /**
   * Stop the server if running
   */
  abstract stopServer?(): Promise<boolean>

  /**
   * Get the current server status
   */
  abstract getServerStatus?(): any

  /**
   * Get server logs
   */
  abstract getServerLogs?(lines?: number): Promise<string>

  /**
   * List files in the workspace
   */
  abstract listFiles(relativePath?: string, recursive?: boolean): Promise<string[]>

  /**
   * Read a file from the workspace
   */
  abstract readFile(filePath: string): Promise<string>

  /**
   * Write a file to the workspace
   */
  abstract writeFile(filePath: string, content: string): Promise<void>

  /**
   * Convert to storage-compatible Workspace object
   */
  abstract toWorkspaceInfo(): Workspace
}
