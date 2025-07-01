import type { RepositoryConfig } from '@shared/types/repository.js'
import type { CommandResult, Workspace, WorkspaceStatus } from '@shared/types/workspace.js'
import type { EventService } from '../services/event.service.js'

export interface FileSearchRequest {
  query: string
  limit?: number
}

export interface FileSearchResult {
  path: string
  absolutePath: string
  score: number
}

export interface FileSearchResponse {
  results: FileSearchResult[]
  totalFound: number
  query: string
}

export abstract class BaseWorkspace {
  protected taskId: string
  protected repositoryConfig: RepositoryConfig
  protected eventService: EventService

  // Properties that match the Workspace interface
  get id(): string {
    return this.taskId
  }

  abstract get type(): 'local' | 'docker'
  abstract get status(): 'initializing' | 'ready' | 'error' | 'cleaning'
  abstract get createdAt(): Date
  abstract get error(): string | undefined

  get repositoryId(): string {
    return this.repositoryConfig.id
  }

  get environmentConfig(): RepositoryConfig {
    return this.repositoryConfig
  }

  constructor(taskId: string, repositoryConfig: RepositoryConfig, eventService: EventService) {
    this.taskId = taskId
    this.repositoryConfig = repositoryConfig
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
  abstract startServer?(): Promise<never>

  /**
   * Stop the server if running
   */
  abstract stopServer?(): Promise<boolean>

  /**
   * Get the current server status
   */
  abstract getServerStatus?(): unknown

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

  /**
   * Get the workspace path (for local workspaces) or container ID (for docker workspaces)
   */
  abstract getWorkspacePath(): string

  /**
   * Check if the workspace physical path or container is valid and accessible
   */
  abstract isWorkspacePathValid(): Promise<boolean>
}
