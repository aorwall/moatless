import type { GitChanges, GitStatus } from '@moatless/types'
import type { RepositoryConfig } from '@moatless/types'
import type { CommandResult, ServerStatus, WorkspaceStatus } from '@moatless/types'
import type { EventService } from '../services/index.js'
import type { GitDiffResponse } from '../utils/index.js'

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
  abstract get status(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning'
  abstract get createdAt(): Date
  abstract get error(): string | undefined

  get repositoryId(): string {
    return this.repositoryConfig.id
  }

  get environmentConfig(): RepositoryConfig {
    return this.repositoryConfig
  }

  protected constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    eventService: EventService,
  ) {
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
  abstract getGitDiff(): Promise<GitDiffResponse>

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
   * Stop the running agent for this workspace
   */
  abstract stopAgent(): Promise<boolean>

  /**
   * Start the server if configured
   */
  abstract startServer?(): Promise<ServerStatus>

  /**
   * Stop the server if running
   */
  abstract stopServer?(): Promise<boolean>

  /**
   * Get the current server status
   */
  abstract getServerStatus?(): Promise<ServerStatus>

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
   * Get the workspace path (for local workspaces) or container ID (for docker workspaces)
   */
  abstract getWorkspacePath(): string

  /**
   * Check if the workspace physical path or container is valid and accessible
   */
  abstract isWorkspacePathValid(): Promise<boolean>

  /**
   * Get git status with additions, deletions, modifications
   */
  abstract getGitStatus(): Promise<GitStatus>

  /**
   * Get working directory changes (not diff to base branch)
   */
  abstract getGitChanges(): Promise<GitChanges>
}
