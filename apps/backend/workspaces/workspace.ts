import type { GitChanges, GitStatus } from '@moatless/types'
import type { RepositoryConfig } from '@moatless/types'
import type { CommandResult, ServerStatus, Workspace as WorkspaceInfo } from '@moatless/types'
import { WorkspaceStatus } from '@moatless/types'
import type { BaseAgent, EventService } from '../services/index.js'
import { FileOperations, type FileSearchRequest, type FileSearchResponse } from '../services/file-operations.service.js'
import type { BaseEnvironment } from '../environments/base-environment.js'
import type { GitDiffResponse } from '../utils/index.js'
import { createTaskLogger } from '../utils/logger.js'

/**
 * Unified Workspace implementation that composes FileOperations and Environment.
 * This replaces the previous LocalWorkspace and DockerWorkspace classes with a single
 * implementation that delegates file operations to FileOperations service and
 * execution operations to the appropriate Environment (Local or Docker).
 */
export class Workspace {
  private fileOperations: FileOperations
  private logger

  // Properties that match the previous BaseWorkspace interface
  get id(): string {
    return this.taskId
  }

  get type(): 'local' | 'docker' {
    return this.environment.getType()
  }

  get status(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    return this.environment.getStatus()
  }

  get createdAt(): Date {
    return this.environment.getCreatedAt()
  }

  get error(): string | undefined {
    return this.environment.getError()
  }

  get repositoryId(): string {
    return this.repositoryConfig.id
  }

  get environmentConfig(): RepositoryConfig {
    return this.repositoryConfig
  }

  get runningAgent(): BaseAgent | undefined {
    return this.environment.isRunning() ? (this.environment as any).getRunningAgent?.() : undefined
  }

  constructor(
    private readonly taskId: string,
    private readonly repositoryConfig: RepositoryConfig,
    _eventService: EventService, // Keep for interface compatibility but not used
    private readonly environment: BaseEnvironment,
    workspacePath?: string,
    originalBranch?: string,
    _branchName?: string, // Keep for interface compatibility but not used
  ) {
    // Initialize logger
    this.logger = createTaskLogger(taskId)
    
    // Initialize FileOperations with the workspace path from environment
    const initialWorkspacePath = workspacePath || environment.getWorkspacePath()
    this.fileOperations = new FileOperations(taskId, initialWorkspacePath, originalBranch)

    this.logger.debug('Workspace constructor', {
      type: this.type,
      repositoryId: repositoryConfig.id,
      repositoryPath: repositoryConfig.path,
      workspacePath: initialWorkspacePath,
      hasOriginalBranch: !!originalBranch,
    })
  }

  /**
   * Set up the workspace (environment setup, then file operations validation)
   */
  async setup(iterationNumber?: number): Promise<void> {
    this.logger.info('Starting Workspace setup', {
      type: this.type,
      repositoryId: this.repositoryConfig.id,
    })

    try {
      // Step 1: Setup the environment (this handles creation of workspace path for LocalEnvironment)
      await this.environment.setup(iterationNumber)

      // Step 2: Update file operations with the resolved workspace path
      const resolvedWorkspacePath = this.environment.getWorkspacePath()
      if (resolvedWorkspacePath) {
        this.fileOperations.setWorkspacePath(resolvedWorkspacePath)
      }

      // Step 3: Validate that workspace path is accessible for file operations
      const isValid = await this.fileOperations.isWorkspacePathValid()
      if (!isValid) {
        throw new Error(`Workspace path is not valid: ${resolvedWorkspacePath}`)
      }

      this.logger.info('Workspace setup completed', {
        type: this.type,
        workspacePath: resolvedWorkspacePath,
      })
    } catch (error) {
      this.logger.error('Workspace setup failed', error as Error, {
        type: this.type,
      })
      throw error
    }
  }

  /**
   * Clean up workspace resources
   */
  async teardown(force = false): Promise<void> {
    const startTime = Date.now()

    this.logger.info('Starting workspace teardown', {
      force,
      type: this.type,
      workspacePath: this.getWorkspacePath(),
    })

    // Check for uncommitted changes first (unless force)
    if (!force) {
      this.logger.debug('Checking for uncommitted changes')
      const hasChanges = await this.hasUncommittedChanges()
      if (hasChanges) {
        this.logger.warn('Teardown aborted due to uncommitted changes')
        throw new Error('Workspace has uncommitted changes. Use force=true to close anyway.')
      }
      this.logger.debug('No uncommitted changes detected')
    }

    // Teardown the environment (will stop agent, server, and clean up environment-specific resources)
    await this.environment.teardown(force)

    const teardownTime = Date.now() - startTime
    this.logger.info('Workspace teardown completed', { teardownTimeMs: teardownTime })
  }

  // ============================================
  // File Operations (delegated to FileOperations)
  // ============================================

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    return this.fileOperations.searchFiles(request)
  }

  async getGitDiff(): Promise<GitDiffResponse> {
    return this.fileOperations.getGitDiff()
  }

  async listFiles(relativePath?: string, recursive?: boolean): Promise<string[]> {
    return this.fileOperations.listFiles(relativePath, recursive)
  }

  async readFile(filePath: string): Promise<string> {
    return this.fileOperations.readFile(filePath)
  }

  async writeFile(filePath: string, content: string): Promise<void> {
    return this.fileOperations.writeFile(filePath, content)
  }

  async hasUncommittedChanges(): Promise<boolean> {
    return this.fileOperations.hasUncommittedChanges()
  }

  async getGitStatus(): Promise<GitStatus> {
    return this.fileOperations.getGitStatus()
  }

  async getGitChanges(): Promise<GitChanges> {
    return this.fileOperations.getGitChanges()
  }

  async getCurrentCommitHash(): Promise<string | null> {
    return this.fileOperations.getCurrentCommitHash()
  }

  getWorkspacePath(): string {
    return this.fileOperations.getWorkspacePath()
  }

  async isWorkspacePathValid(): Promise<boolean> {
    return this.fileOperations.isWorkspacePathValid()
  }

  // ============================================
  // Environment Operations (delegated to Environment)
  // ============================================

  async runCommand(command: string, timeout?: number): Promise<CommandResult> {
    return this.environment.runCommand(command, timeout)
  }

  isRunning(): boolean {
    return this.environment.isRunning()
  }

  async stopAgent(): Promise<boolean> {
    return this.environment.stopAgent()
  }

  // Server methods (optional - only available if environment supports them)
  async startServer(): Promise<ServerStatus> {
    if (!this.environment.startServer) {
      throw new Error(`Server operations not supported for ${this.type} environment`)
    }
    return this.environment.startServer()
  }

  async stopServer(): Promise<boolean> {
    if (!this.environment.stopServer) {
      throw new Error(`Server operations not supported for ${this.type} environment`)
    }
    return this.environment.stopServer()
  }

  async getServerStatus(): Promise<ServerStatus> {
    if (!this.environment.getServerStatus) {
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }
    return this.environment.getServerStatus()
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (!this.environment.getServerLogs) {
      return 'Server logs not available for this environment type'
    }
    return this.environment.getServerLogs(lines)
  }

  // ============================================
  // Status and Utility Methods
  // ============================================

  getStatus(): WorkspaceStatus {
    // Check if agent is running
    if (this.runningAgent) {
      return WorkspaceStatus.RUNNING
    }

    // Map environment status to workspace status
    const envStatus = this.environment.getStatus()
    switch (envStatus) {
      case 'not_setup':
        return WorkspaceStatus.NOT_SETUP
      case 'initializing':
        return WorkspaceStatus.SETTING_UP
      case 'ready':
        return WorkspaceStatus.IDLE
      case 'error':
      case 'cleaning':
        return WorkspaceStatus.IDLE // Map error/cleaning to idle for backwards compatibility
      default:
        return WorkspaceStatus.NOT_SETUP
    }
  }

  /**
   * Convert to workspace info for API responses
   */
  toWorkspaceInfo(): WorkspaceInfo {
    const baseInfo = {
      id: this.taskId,
      repositoryId: this.repositoryConfig.id,
      status: this.status,
      createdAt: this.createdAt,
      error: this.error,
    }

    if (this.type === 'local') {
      return {
        ...baseInfo,
        type: 'local' as const,
        path: this.getWorkspacePath(),
      }
    } else {
      return {
        ...baseInfo,
        type: 'docker' as const,
        containerId: (this.environment as any).getContainerId?.() || '',
        port: (this.environment as any).getPort?.(),
        image: (this.environment as any).getImage?.(),
      }
    }
  }

  /**
   * Set running agent (delegated to environment if it supports it)
   */
  setRunningAgent(agent: BaseAgent): void {
    if ((this.environment as any).setRunningAgent) {
      ;(this.environment as any).setRunningAgent(agent)
    }
  }

  /**
   * Get the environment instance
   */
  getEnvironment(): BaseEnvironment {
    return this.environment
  }
}