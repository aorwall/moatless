import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { RepositoryConfig, ServerStatus, WorkspaceStatusResponse } from '@moatless/types'
import { RepositoryNotFoundError, TaskNotFoundError, WorkspaceError } from '@moatless/types'
import { DockerEnvironment } from '../environments/docker-environment.js'
import { LocalEnvironment } from '../environments/local-environment.js'
import { createGitWorktree, getBranchList, isGitRepository, isGitWorktree } from '../utils/index.js'
import { logger } from '../utils/logger.js'
import { Workspace } from '../workspaces/workspace.js'
import type { EventService } from './event.service.js'
import type { PreviewEnvironmentService } from './preview-environment.service.js'
import type { RepositoryService } from './repository.service.js'
import type { SettingsService } from './settings.service.js'
import type { StorageBackend } from './storage.js'

export class WorkspaceService {
  private activeWorkspaces: Map<string, Workspace> = new Map()
  private backgroundTasks: Map<string, Set<Promise<unknown>>> = new Map()
  private previewEnvironmentService?: PreviewEnvironmentService

  constructor(
    private repositoryService: RepositoryService,
    private storage: StorageBackend,
    private settingsService: SettingsService,
    private eventService: EventService,
  ) {}

  async createWorkspace(
    taskId: string,
    repositoryId: string,
    branchName?: string,
  ): Promise<Workspace> {
    logger.info('Starting workspace creation', { taskId, repositoryId, branchName })

    const repository = await this.repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      logger.error('Repository not found', undefined, { repositoryId })
      throw new RepositoryNotFoundError(repositoryId)
    }

    logger.debug('Repository loaded', {
      taskId,
      repositoryName: repository.name,
      codeExecutionType: repository.codeExecution.type,
      useGitWorktrees: repository.useGitWorktrees,
    })

    // Create workspace path for local repositories
    const workspacePath = await this.createWorkspaceRepository(repository, taskId, branchName)

    logger.info('Workspace repository created', {
      taskId,
      workspacePath,
      branchName,
    })

    // Create the appropriate environment based on repository config
    let environment: DockerEnvironment | LocalEnvironment
    if (repository.codeExecution.type === 'docker') {
      logger.info('Creating Docker environment', { taskId, workspacePath })
      environment = new DockerEnvironment(
        taskId,
        repository,
        workspacePath,
        this.settingsService,
        this.eventService,
      )
    } else {
      logger.info('Creating Local environment', { taskId, workspacePath })
      environment = new LocalEnvironment(taskId, repository, workspacePath, this.eventService)
    }

    // Create unified workspace that composes the environment and file operations
    const workspace = new Workspace(taskId, repository, environment, workspacePath)

    this.activeWorkspaces.set(taskId, workspace)

    logger.info('Workspace created successfully', {
      taskId,
      workspaceType: environment.getType(),
      workspacePath,
    })

    return workspace
  }

  private async createWorkspaceRepository(
    repo: RepositoryConfig,
    taskId: string,
    branchName?: string,
  ): Promise<string> {
    if (!(await this.pathExists(repo.path))) {
      throw new WorkspaceError(`Repository path does not exist: ${repo.path}`, taskId, 400)
    }

    // If worktrees are disabled, use repository path directly
    if (!repo.useGitWorktrees) {
      return repo.path
    }

    // Create workspace path for git worktrees
    const worktreeDir = this.getWorktreeDir(repo)
    const workspacePath = path.join(worktreeDir, this.getWorktreeFolderName(repo, taskId))

    logger.info('Using git worktree setup', { workspacePath, taskId })

    // Verify repo is a git repository before creating worktree
    if (!(await isGitRepository(repo.path))) {
      throw new WorkspaceError(`Repository path ${repo.path} is not a git repository`, taskId, 400)
    }

    // Create worktree if it doesn't exist or recreate if removed
    await this.ensureWorktreeExists(repo, taskId, workspacePath, branchName)

    return workspacePath
  }

  private getBranchName(taskId: string): string {
    return `workspace-${taskId}`
  }

  private getWorktreeFolderName(repo: RepositoryConfig, taskId: string): string {
    // Get the last directory name from the repo path
    const repoLastDirName = path.basename(repo.path)

    // Use the branch name for the worktree folder
    return `${repoLastDirName}-${this.getBranchName(taskId)}`
  }

  // Private helper methods
  private getWorktreeDir(repo: RepositoryConfig): string {
    // If worktrees are disabled, use repository path directly
    if (!repo.useGitWorktrees) {
      return repo.path
    }

    // Check for environment variable first
    const worktreeDir = process.env.WORKTREE_DIR
    if (worktreeDir) {
      return worktreeDir
    }

    // Default to "../" relative to repo path
    return path.join(repo.path, '..')
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  /**
   * Ensures that a worktree exists at the given path, recreating it if necessary.
   * This handles cases where the worktree directory was removed externally (e.g., git worktree remove).
   */
  private async ensureWorktreeExists(
    repo: RepositoryConfig,
    taskId: string,
    workspacePath: string,
    branchName?: string,
  ): Promise<void> {
    const workspaceLogger = logger.child({ taskId })

    // Check if workspace path exists and is a valid git worktree
    const pathExists = await this.pathExists(workspacePath)
    const isValidWorktree = pathExists && (await isGitWorktree(workspacePath))

    if (isValidWorktree) {
      workspaceLogger.debug('Worktree already exists and is valid', { workspacePath })
      return
    }

    if (pathExists && !isValidWorktree) {
      workspaceLogger.warn('Workspace path exists but is not a valid worktree, will recreate', {
        workspacePath,
      })
    } else {
      workspaceLogger.info('Worktree does not exist, creating new one', { workspacePath })
    }

    // Get the workspace branch name
    const workspaceBranchName = this.getBranchName(taskId)

    // Check if the workspace branch already exists
    const workspaceBranchExists = await this.checkBranchExists(repo.path, workspaceBranchName)

    if (workspaceBranchExists) {
      // The branch exists, try to use it directly without -b flag
      workspaceLogger.info('Workspace branch already exists, reusing it', {
        branch: workspaceBranchName,
      })
      try {
        // Create worktree using existing branch (no -b flag)
        await createGitWorktree(repo.path, workspacePath, workspaceBranchName)
      } catch (error) {
        workspaceLogger.error('Failed to create worktree with existing branch', error as Error, {
          branch: workspaceBranchName,
          workspacePath,
        })
        throw error
      }
    } else if (branchName) {
      // Check if source branch exists
      const branchExists = await this.checkBranchExists(repo.path, branchName)
      if (!branchExists) {
        throw new WorkspaceError(`Branch '${branchName}' does not exist in repository`, taskId, 400)
      }

      // Create new branch based on the provided branch
      await createGitWorktree(repo.path, workspacePath, branchName, workspaceBranchName)
    } else {
      // Create a new branch for this task
      await createGitWorktree(repo.path, workspacePath, undefined, workspaceBranchName)
    }

    workspaceLogger.info('Worktree created successfully', { workspacePath, branchName })
  }

  /**
   * Checks if a branch exists in the repository
   */
  private async checkBranchExists(repoPath: string, branchName: string): Promise<boolean> {
    try {
      const branches = await getBranchList(repoPath)
      return branches.includes(branchName)
    } catch (error) {
      logger.error('Failed to check branch existence', error as Error, {
        repoPath,
        branchName,
      })
      return false
    }
  }

  /**
   * Gets the branch name from the last completed iteration, or null if none exists
   */
  private async getLastIterationBranch(taskId: string): Promise<string | null> {
    try {
      const task = await this.storage.getTask(taskId)
      if (!task || !task.iterations || task.iterations.length === 0) {
        return null
      }

      // Find the last completed iteration with git status
      const lastCompletedIteration = task.iterations
        .filter(
          (iteration) => iteration.status === 'completed' && iteration.gitStatus?.currentBranch,
        )
        .sort((a, b) => b.iterationNumber - a.iterationNumber)[0]

      if (lastCompletedIteration?.gitStatus?.currentBranch) {
        logger.debug('Found branch from last completed iteration', {
          taskId,
          iterationNumber: lastCompletedIteration.iterationNumber,
          branch: lastCompletedIteration.gitStatus.currentBranch,
        })
        return lastCompletedIteration.gitStatus.currentBranch
      }

      return null
    } catch (error) {
      logger.error('Failed to get last iteration branch', error as Error, { taskId })
      return null
    }
  }

  /**
   * Removes a worktree directory if it exists and worktrees are enabled.
   * This should only be called when tasks are closed or deleted, not during regular teardown.
   */
  async removeWorktree(taskId: string): Promise<void> {
    const workspaceLogger = logger.child({ taskId })

    try {
      // Get the task to find the workspace path
      const task = await this.storage.getTask(taskId)
      if (!task || !task.workspacePath) {
        workspaceLogger.debug('No workspace path found for task, skipping worktree removal')
        return
      }

      const workspacePath = task.workspacePath
      await this.removeWorktreeIfExists(taskId, workspacePath)
    } catch (error) {
      workspaceLogger.error('Failed to remove worktree', error as Error, { taskId })
      // Don't rethrow - worktree removal failure shouldn't block task operations
    }
  }

  /**
   * Internal helper to remove a worktree directory if it exists and worktrees are enabled
   */
  private async removeWorktreeIfExists(taskId: string, workspacePath: string): Promise<void> {
    if (!workspacePath) {
      return
    }

    const workspaceLogger = logger.child({ taskId })

    try {
      // Check if path exists
      if (!(await this.pathExists(workspacePath))) {
        workspaceLogger.debug('Worktree path does not exist, skipping removal', { workspacePath })
        return
      }

      // Check if it's a git worktree
      if (!(await isGitWorktree(workspacePath))) {
        workspaceLogger.debug('Path is not a git worktree, skipping removal', { workspacePath })
        return
      }

      // Get the repository path to run git worktree remove
      const task = await this.storage.getTask(taskId)
      if (!task) {
        workspaceLogger.warn('Task not found, cannot determine repository for worktree removal', {
          workspacePath,
        })
        return
      }

      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        workspaceLogger.warn('Repository not found, cannot remove worktree', {
          repositoryId: task.repositoryId,
          workspacePath,
        })
        return
      }

      // Only remove if worktrees are enabled for this repository
      if (!repository.useGitWorktrees) {
        workspaceLogger.debug('Git worktrees disabled for repository, skipping removal', {
          repositoryId: repository.id,
          workspacePath,
        })
        return
      }

      // Remove the worktree using git command
      workspaceLogger.info('Removing git worktree', { workspacePath })

      const { exec } = await import('node:child_process')
      const { promisify } = await import('node:util')
      const execAsync = promisify(exec)

      await execAsync(`git worktree remove "${workspacePath}" --force`, {
        cwd: repository.path,
        timeout: 10000,
      })

      workspaceLogger.info('Git worktree removed successfully', { workspacePath })
    } catch (error) {
      workspaceLogger.error('Failed to remove git worktree', error as Error, {
        workspacePath,
        // Continue with cleanup even if worktree removal fails
      })
    }
  }

  async getOrCreateWorkspace(taskId: string, branchName?: string): Promise<Workspace> {
    const workspaceLogger = logger.child({ taskId })

    // Check if workspace already exists
    const existing = this.activeWorkspaces.get(taskId)
    if (existing) {
      workspaceLogger.debug('Workspace already exists, returning existing instance', {
        status: existing.getStatus(),
      })
      return existing
    }

    // Get task to retrieve repository ID and workspace path
    const task = await this.storage.getTask(taskId)
    if (!task) {
      workspaceLogger.error('Task not found', undefined, { taskId })
      throw new TaskNotFoundError(taskId)
    }

    const repositoryId = task.repositoryId
    const existingWorkspacePath = task.workspacePath

    workspaceLogger.debug('Retrieved task data', {
      repositoryId,
      hasExistingWorkspacePath: !!existingWorkspacePath,
    })

    // Get repository
    const repository = await this.repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      workspaceLogger.error('Repository not found', undefined, { repositoryId })
      throw new RepositoryNotFoundError(repositoryId)
    }

    workspaceLogger.debug('Repository loaded successfully', {
      repositoryPath: repository.path,
      codeExecutionType: repository.codeExecution.type,
      useGitWorktrees: repository.useGitWorktrees || false,
    })

    try {
      // For existing workspaces, validate the workspace path and recreate if necessary
      let workspacePath: string
      if (existingWorkspacePath) {
        // Check if the existing workspace path is still valid
        if (repository.useGitWorktrees && !(await isGitWorktree(existingWorkspacePath))) {
          workspaceLogger.warn('Existing workspace path is not a valid worktree, recreating', {
            existingPath: existingWorkspacePath,
          })

          // Try to get the branch from the last completed iteration
          const lastIterationBranch = await this.getLastIterationBranch(taskId)
          const branchToUse = branchName || lastIterationBranch || undefined

          workspaceLogger.debug('Using branch for worktree recreation', {
            providedBranch: branchName,
            lastIterationBranch,
            branchToUse,
          })

          // Recreate the worktree at the same path
          await this.ensureWorktreeExists(repository, taskId, existingWorkspacePath, branchToUse)
        } else if (!repository.useGitWorktrees && !(await this.pathExists(existingWorkspacePath))) {
          throw new WorkspaceError(
            `Workspace path does not exist and worktrees are disabled: ${existingWorkspacePath}`,
            taskId,
            400,
          )
        }
        workspacePath = existingWorkspacePath
      } else {
        // Create new workspace path
        workspacePath = await this.createWorkspaceRepository(repository, taskId, branchName)
      }

      // Create the appropriate environment based on repository config
      let environment: DockerEnvironment | LocalEnvironment
      if (repository.codeExecution.type === 'docker') {
        workspaceLogger.debug('Creating Docker environment with workspace path', { workspacePath })
        environment = new DockerEnvironment(
          taskId,
          repository,
          workspacePath,
          this.settingsService,
          this.eventService,
        )
      } else {
        workspaceLogger.debug('Creating Local environment with workspace path', {
          workspacePath,
          branchName,
          existingWorkspacePath,
        })
        environment = new LocalEnvironment(taskId, repository, workspacePath, this.eventService)
      }

      // Create unified workspace that composes the environment and file operations
      const workspace = new Workspace(taskId, repository, environment, workspacePath)

      // Store the workspace in the active workspaces map
      this.activeWorkspaces.set(taskId, workspace)

      return workspace
    } catch (error) {
      workspaceLogger.error('Workspace creation failed', error as Error, {
        repositoryPath: repository.path,
        codeExecutionType: repository.codeExecution.type,
      })
      throw error
    }
  }

  async getWorkspace(taskId: string): Promise<Workspace | null> {
    // Check active workspaces only
    return this.activeWorkspaces.get(taskId) || null
  }

  async canManageServer(taskId: string): Promise<boolean> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      const status = workspace.getEnvironment().getStatus()
      // Server can be managed when workspace environment is not yet ready
      return status === 'ready'
    } catch {
      return false
    }
  }
  async cleanupWorkspace(taskId: string): Promise<void> {
    const workspaceLogger = logger.child({ taskId })

    workspaceLogger.info('Starting workspace cleanup')

    const workspace = this.activeWorkspaces.get(taskId)
    if (!workspace) {
      workspaceLogger.debug('No workspace found for cleanup')
      return
    }

    const startTime = Date.now()
    try {
      // Get workspace details before cleanup for logging
      const workspacePath = workspace.getWorkspacePath()
      const workspaceStatus = workspace.getStatus()

      workspaceLogger.info('Initiating workspace teardown', {
        workspaceType: workspace.type,
        workspacePath,
        workspaceStatus,
        force: true,
      })

      await workspace.teardown(true) // Force cleanup

      const teardownTime = Date.now() - startTime
      workspaceLogger.info('Workspace teardown completed', {
        teardownTimeMs: teardownTime,
        workspacePath,
      })

      // Remove from active workspaces
      this.activeWorkspaces.delete(taskId)
      workspaceLogger.debug('Workspace removed from active workspaces map')

      workspaceLogger.info('Workspace cleanup completed successfully', {
        totalTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      workspaceLogger.error('Workspace cleanup failed', error as Error, {
        timeToFailureMs: failureTime,
        workspaceType: workspace.type,
      })
      // Don't rethrow - we want to try to clean up as much as possible
      // Still remove from active workspaces even if teardown failed
      this.activeWorkspaces.delete(taskId)
    }
  }

  async setupWorkspace(taskId: string, iterationNumber?: number): Promise<void> {
    logger.info('Starting workspace setup', { taskId, iterationNumber })

    const workspace = await this.getOrCreateWorkspace(taskId)

    logger.debug('Workspace retrieved, starting setup', {
      taskId,
      workspaceType: workspace.type,
      currentStatus: workspace.getStatus(),
    })

    await workspace.setup(iterationNumber)

    logger.info('Workspace setup completed', {
      taskId,
      workspaceStatus: workspace.getStatus(),
    })

    // Save the workspace path to the task after successful setup
    const workspacePath = workspace.getWorkspacePath()
    if (workspacePath) {
      const task = await this.storage.getTask(taskId)
      if (task && !task.workspacePath) {
        task.workspacePath = workspacePath
        await this.storage.saveTask(task)
        logger.debug('Saved workspace path to task', { taskId, workspacePath })
      }
    } else {
      logger.warn('Workspace path not available after setup', { taskId })
    }
  }

  async getWorkspaceStatus(taskId: string): Promise<WorkspaceStatusResponse> {
    // Validate task exists first
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    const workspace = await this.getOrCreateWorkspace(taskId)
    const backgroundTaskCount = this.backgroundTasks.get(taskId)?.size || 0

    // Delegate to workspace method
    return workspace.getWorkspaceStatus(backgroundTaskCount)
  }

  async startServer(taskId: string): Promise<ServerStatus> {
    if (!(await this.canManageServer(taskId))) {
      throw new WorkspaceError('Cannot start server: workspace not ready', taskId)
    }

    // Get workspace to check if environment supports server operations
    const workspace = await this.getOrCreateWorkspace(taskId)
    const environment = workspace.getEnvironment()

    // Check if environment has server methods and delegate to it
    if (environment.startServer) {
      logger.info('Delegating server start to environment', {
        taskId,
        environmentType: environment.getType(),
      })
      return environment.startServer()
    }

    // Fallback to preview environment service for backwards compatibility
    if (!this.previewEnvironmentService) {
      throw new WorkspaceError('Preview environment service not initialized', taskId)
    }

    return this.previewEnvironmentService.startServer(taskId)
  }

  async stopServer(taskId: string): Promise<boolean> {
    if (!(await this.canManageServer(taskId))) {
      throw new WorkspaceError('Cannot stop server: workspace not ready', taskId)
    }

    // Get workspace to check if environment supports server operations
    const workspace = await this.getOrCreateWorkspace(taskId)
    const environment = workspace.getEnvironment()

    // Check if environment has server methods and delegate to it
    if (environment.stopServer) {
      logger.info('Delegating server stop to environment', {
        taskId,
        environmentType: environment.getType(),
      })
      return environment.stopServer()
    }

    // Fallback to preview environment service for backwards compatibility
    if (!this.previewEnvironmentService) {
      throw new WorkspaceError('Preview environment service not initialized', taskId)
    }

    return this.previewEnvironmentService.stopServer(taskId)
  }

  async restartServer(taskId: string): Promise<ServerStatus> {
    if (!(await this.canManageServer(taskId))) {
      throw new WorkspaceError('Cannot restart server: workspace not ready', taskId)
    }

    // For restart, we can simply call stop then start
    await this.stopServer(taskId)
    // Wait a moment for the server to fully stop
    await new Promise((resolve) => setTimeout(resolve, 1000))
    return this.startServer(taskId) as Promise<ServerStatus>
  }

  async getServerStatus(taskId: string): Promise<ServerStatus> {
    try {
      // Get workspace to check if environment supports server operations
      const workspace = await this.getOrCreateWorkspace(taskId)
      const environment = workspace.getEnvironment()

      // Check if environment has server methods and delegate to it
      if (environment.getServerStatus) {
        logger.debug('Delegating server status to environment', {
          taskId,
          environmentType: environment.getType(),
        })
        return environment.getServerStatus()
      }

      // Fallback to preview environment service for backwards compatibility
      if (!this.previewEnvironmentService) {
        return {
          running: false,
          configured: false,
          uptimeSeconds: 0,
        }
      }

      return this.previewEnvironmentService.getServerStatus(taskId)
    } catch (error) {
      logger.error('Error getting server status', error as Error, { taskId })
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }
  }

  // Background task tracking methods
  public trackBackgroundTask(taskId: string, task: Promise<unknown>): void {
    if (!this.backgroundTasks.has(taskId)) {
      this.backgroundTasks.set(taskId, new Set())
    }
    const taskSet = this.backgroundTasks.get(taskId)!
    taskSet.add(task)

    // Clean up when task completes
    task.finally(() => {
      taskSet.delete(task)
      if (taskSet.size === 0) {
        this.backgroundTasks.delete(taskId)
      }
    })
  }

  async getServerLogs(taskId: string, lines?: number): Promise<string> {
    if (!(await this.canManageServer(taskId))) {
      return 'Server logs not available: workspace not ready'
    }

    try {
      // Get workspace to check if environment supports server operations
      const workspace = await this.getOrCreateWorkspace(taskId)
      const environment = workspace.getEnvironment()

      // Check if environment has server methods and delegate to it
      if (environment.getServerLogs) {
        logger.debug('Delegating server logs to environment', {
          taskId,
          environmentType: environment.getType(),
        })
        return environment.getServerLogs(lines)
      }

      // Fallback to preview environment service for backwards compatibility
      if (!this.previewEnvironmentService) {
        return 'Preview environment service not initialized'
      }

      return this.previewEnvironmentService.getServerLogs(taskId, lines)
    } catch (error) {
      logger.error('Error getting server logs', error as Error, { taskId })
      return `Error getting server logs: ${error instanceof Error ? error.message : String(error)}`
    }
  }

  setPreviewEnvironmentService(previewEnvironmentService: PreviewEnvironmentService): void {
    this.previewEnvironmentService = previewEnvironmentService
  }

  async gracefulShutdown(): Promise<void> {
    const workspaceLogger = logger.child({ context: 'graceful-shutdown' })
    workspaceLogger.info('Starting workspace service graceful shutdown')

    const activeWorkspaceIds = Array.from(this.activeWorkspaces.keys())
    workspaceLogger.info(`Cleaning up ${activeWorkspaceIds.length} active workspaces`)

    // Clean up all active workspaces concurrently
    const cleanupPromises = activeWorkspaceIds.map(async (taskId) => {
      try {
        await this.cleanupWorkspace(taskId)
      } catch (error) {
        workspaceLogger.error(`Failed to cleanup workspace ${taskId}`, error as Error)
        // Continue with other cleanups even if one fails
      }
    })

    await Promise.allSettled(cleanupPromises)

    // Clear background tasks
    this.backgroundTasks.clear()

    workspaceLogger.info('Workspace service graceful shutdown completed')
  }
}

export function createWorkspaceService(
  repositoryService: RepositoryService,
  storage: StorageBackend,
  settingsService: SettingsService,
  eventService: EventService,
): WorkspaceService {
  return new WorkspaceService(repositoryService, storage, settingsService, eventService)
}
