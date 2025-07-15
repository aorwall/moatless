import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CommandResult,
  FileTreeNode,
  FileTreeWithChangesResponse,
  RepositoryConfig,
  ServerStatus,
} from '@moatless/types'
import {
  RepositoryNotFoundError,
  TaskNotFoundError,
  WorkspaceError,
  WorkspaceStatus,
} from '@moatless/types'
import { DockerEnvironment } from '../environments/docker-environment.js'
import { LocalEnvironment } from '../environments/local-environment.js'
import type { FileSearchRequest, FileSearchResponse } from '../services/file-operations.service.js'
import type { GitDiffResponse } from '../utils/index.js'
import {
  createGitWorktree,
  getBranchList,
  isGitRepository,
  isGitWorktree,
  syncWithMain,
} from '../utils/index.js'
import { logger } from '../utils/logger.js'
import { Workspace } from '../workspaces/workspace.js'
import type { EventService } from './event.service.js'
import type { PreviewEnvironmentService } from './preview-environment.service.js'
import type { RepositoryService } from './repository.service.js'
import type { StorageBackend } from './storage.js'

export class WorkspaceService {
  private activeWorkspaces: Map<string, Workspace> = new Map()
  private backgroundTasks: Map<string, Set<Promise<unknown>>> = new Map()
  private previewEnvironmentService?: PreviewEnvironmentService

  constructor(
    private repositoryService: RepositoryService,
    private eventService: EventService,
    private storage: StorageBackend,
  ) {}

  async createWorkspace(
    taskId: string,
    repositoryId: string,
    branchName?: string,
  ): Promise<Workspace> {
    const repository = await this.repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      logger.error('Repository not found', undefined, { repositoryId })
      throw new RepositoryNotFoundError(repositoryId)
    }

    // Create workspace path for local repositories
    const workspacePath = await this.createWorkspaceRepository(repository, taskId, branchName)

    // Create the appropriate environment based on repository config
    let environment
    if (repository.codeExecution.type === 'docker') {
      environment = new DockerEnvironment(taskId, repository, this.eventService, workspacePath)
    } else {
      environment = new LocalEnvironment(taskId, repository, this.eventService, workspacePath)
    }

    // Create unified workspace that composes the environment and file operations
    const workspace = new Workspace(
      taskId,
      repository,
      this.eventService,
      environment,
      workspacePath,
      undefined, // originalBranch
      branchName,
    )

    this.activeWorkspaces.set(taskId, workspace)

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

    // If branchName is provided, check if it exists
    if (branchName) {
      const branchExists = await this.checkBranchExists(repo.path, branchName)
      if (!branchExists) {
        throw new WorkspaceError(`Branch '${branchName}' does not exist in repository`, taskId, 400)
      }

      // When using an existing branch, we might need to create a new branch
      // if the existing one is already checked out
      const newBranchName = this.getBranchName(taskId)
      await createGitWorktree(repo.path, workspacePath, branchName, newBranchName)
    } else {
      // Create a new branch for this task
      const newBranchName = this.getBranchName(taskId)
      await createGitWorktree(repo.path, workspacePath, undefined, newBranchName)
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
      let environment
      if (repository.codeExecution.type === 'docker') {
        workspaceLogger.debug('Creating Docker environment with workspace path', { workspacePath })
        environment = new DockerEnvironment(taskId, repository, this.eventService, workspacePath)
      } else {
        workspaceLogger.debug('Creating Local environment with workspace path', {
          workspacePath,
          branchName,
          existingWorkspacePath,
        })
        environment = new LocalEnvironment(taskId, repository, this.eventService, workspacePath)
      }

      // Create unified workspace that composes the environment and file operations
      const workspace = new Workspace(
        taskId,
        repository,
        this.eventService,
        environment,
        workspacePath,
        undefined, // originalBranch
        branchName,
      )

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

  // Workspace capability methods
  async canListFiles(taskId: string): Promise<boolean> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      const status = workspace.getStatus()
      // Files can be listed if workspace exists and is at least setting up
      return status !== WorkspaceStatus.NOT_SETUP
    } catch {
      return false
    }
  }

  async canReadWriteFiles(taskId: string): Promise<boolean> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      const status = workspace.getStatus()
      // Files can be read/written if workspace is setting up, idle, or running
      return status !== WorkspaceStatus.NOT_SETUP
    } catch {
      return false
    }
  }

  async canExecuteCommands(taskId: string): Promise<boolean> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      const status = workspace.getStatus()
      // Commands can only be executed when workspace is idle
      return status === WorkspaceStatus.IDLE
    } catch {
      return false
    }
  }

  async canManageServer(taskId: string): Promise<boolean> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      const status = workspace.getStatus()
      // Server can be managed when workspace is idle or running
      return status === WorkspaceStatus.IDLE || status === WorkspaceStatus.RUNNING
    } catch {
      return false
    }
  }

  async canAccessGit(taskId: string): Promise<boolean> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      const status = workspace.getStatus()
      // Git operations available when workspace exists and is at least setting up
      return status !== WorkspaceStatus.NOT_SETUP
    } catch {
      return false
    }
  }

  async listFiles(taskId: string, relativePath = '.', recursive = false): Promise<string[]> {
    // Task validation is now handled in canListFiles, but we still check capability
    if (!(await this.canListFiles(taskId))) {
      throw new WorkspaceError('Cannot list files: workspace not ready', taskId)
    }
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.listFiles(relativePath, recursive)
  }

  async readFile(taskId: string, filePath: string): Promise<string> {
    if (!(await this.canReadWriteFiles(taskId))) {
      throw new WorkspaceError('Cannot read file: workspace not ready', taskId)
    }
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.readFile(filePath)
  }

  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    if (!(await this.canReadWriteFiles(taskId))) {
      throw new WorkspaceError('Cannot write file: workspace not ready', taskId)
    }
    const workspace = await this.getOrCreateWorkspace(taskId)
    await workspace.writeFile(filePath, content)
  }

  async getGitDiff(taskId: string): Promise<GitDiffResponse> {
    if (!(await this.canAccessGit(taskId))) {
      throw new WorkspaceError('Cannot access git: workspace not ready', taskId)
    }
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.getGitDiff()
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
    const workspace = await this.getOrCreateWorkspace(taskId)
    await workspace.setup(iterationNumber)

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

  async runCommand(taskId: string, command: string, timeout?: number): Promise<CommandResult> {
    if (!(await this.canExecuteCommands(taskId))) {
      throw new WorkspaceError('Cannot execute command: workspace not in idle state', taskId)
    }
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.runCommand(command, timeout)
  }

  async getWorkspaceStatus(taskId: string): Promise<unknown> {
    // Validate task exists first
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    const workspace = await this.getOrCreateWorkspace(taskId)
    const backgroundTaskCount = this.backgroundTasks.get(taskId)?.size || 0

    // Get git status and git changes from workspace
    const [gitStatus, gitChanges] = await Promise.all([
      workspace.getGitStatus(),
      workspace.getGitChanges(),
    ])

    // Get environment information
    const environment = workspace.getEnvironment()
    const environmentType = environment.getType()
    const environmentStatus = environment.getStatus()
    const environmentError = environment.getError()
    const environmentCreatedAt = environment.getCreatedAt()

    // Check if file operations are available (workspace path exists on disk)
    const fileOperationsAvailable = await workspace.isWorkspacePathValid()

    // Determine capabilities based on environment status and file operations availability
    const capabilities = {
      fileOperations: fileOperationsAvailable,
      commandExecution: environmentStatus === 'ready',
    }

    return {
      status: workspace.getStatus(),
      gitStatus,
      gitChanges,
      backgroundTaskCount,
      environment: {
        type: environmentType,
        status: environmentStatus,
        error: environmentError,
        createdAt: environmentCreatedAt.toISOString(),
        capabilities,
      },
    }
  }

  async startServer(taskId: string): Promise<unknown> {
    if (!(await this.canManageServer(taskId))) {
      throw new WorkspaceError('Cannot start server: workspace not ready', taskId)
    }

    if (!this.previewEnvironmentService) {
      throw new WorkspaceError('Preview environment service not initialized', taskId)
    }

    return this.previewEnvironmentService.startServer(taskId)
  }

  async stopServer(taskId: string): Promise<boolean> {
    if (!(await this.canManageServer(taskId))) {
      throw new WorkspaceError('Cannot stop server: workspace not ready', taskId)
    }

    if (!this.previewEnvironmentService) {
      throw new WorkspaceError('Preview environment service not initialized', taskId)
    }

    return this.previewEnvironmentService.stopServer(taskId)
  }

  async restartServer(taskId: string): Promise<ServerStatus> {
    if (!(await this.canManageServer(taskId))) {
      throw new WorkspaceError('Cannot restart server: workspace not ready', taskId)
    }

    if (!this.previewEnvironmentService) {
      throw new WorkspaceError('Preview environment service not initialized', taskId)
    }

    return this.previewEnvironmentService.restartServer(taskId)
  }

  async getServerStatus(taskId: string): Promise<ServerStatus> {
    if (!this.previewEnvironmentService) {
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }

    return this.previewEnvironmentService.getServerStatus(taskId)
  }

  async proxyPreviewRequest(
    taskId: string,
    request: {
      method: string
      path: string
      headers: Headers
      body?: string
    },
  ): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }> {
    if (!(await this.canManageServer(taskId))) {
      throw new WorkspaceError('Preview capability not available for this task', taskId)
    }

    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.proxyPreviewRequest(request)
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

  async searchFiles(taskId: string, request: FileSearchRequest): Promise<FileSearchResponse> {
    if (!(await this.canSearchFiles(taskId))) {
      throw new WorkspaceError('Cannot search files: workspace not ready', taskId)
    }
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.searchFiles(request)
  }

  async canSearchFiles(taskId: string): Promise<boolean> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      const status = workspace.getStatus()
      // Files can be searched if workspace exists and is at least setting up
      return status !== WorkspaceStatus.NOT_SETUP
    } catch {
      return false
    }
  }

  async getServerLogs(taskId: string, lines?: number): Promise<string> {
    if (!(await this.canManageServer(taskId))) {
      return 'Server logs not available: workspace not ready'
    }

    if (!this.previewEnvironmentService) {
      return 'Preview environment service not initialized'
    }

    return this.previewEnvironmentService.getServerLogs(taskId, lines)
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

  async syncWorkspaceWithMain(
    taskId: string,
    baseBranch?: string,
  ): Promise<{
    success: boolean
    conflicts: boolean
    message: string
    commitsBehind?: number
    commitsAhead?: number
  }> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    const workspacePath = workspace.getWorkspacePath()

    if (!workspacePath) {
      throw new WorkspaceError('Workspace path not available', taskId)
    }

    if (!(await this.canAccessGit(taskId))) {
      throw new WorkspaceError('Cannot access git: workspace not ready', taskId)
    }

    try {
      const result = await syncWithMain(workspacePath, baseBranch)

      logger.info('Workspace sync completed', {
        taskId,
        workspacePath,
        success: result.success,
        conflicts: result.conflicts,
        commitsBehind: result.commitsBehind,
        commitsAhead: result.commitsAhead,
      })

      return result
    } catch (error) {
      logger.error('Failed to sync workspace with main', error as Error, { taskId, workspacePath })
      throw new WorkspaceError(
        `Sync failed: ${error instanceof Error ? error.message : String(error)}`,
        taskId,
      )
    }
  }

  /**
   * Get file tree with git change information
   * This combines file listing and git diff data in a single performant call
   */
  async getFileTreeWithChanges(
    taskId: string,
    rootPath = '.',
  ): Promise<FileTreeWithChangesResponse> {
    if (!(await this.canListFiles(taskId))) {
      throw new WorkspaceError('Cannot list files: workspace not ready', taskId)
    }

    const workspace = await this.getOrCreateWorkspace(taskId)
    const workspacePath = workspace.getWorkspacePath()

    if (!workspacePath) {
      throw new WorkspaceError('Workspace path not available', taskId)
    }

    try {
      // Parallel execution for performance
      const [gitDiff, allFiles] = await Promise.all([
        workspace.getGitDiff(),
        this.getAllFiles(workspacePath, rootPath),
      ])

      // Create a map of file paths to git diff information for O(1) lookup
      const gitFileMap = new Map<
        string,
        {
          status: 'added' | 'modified' | 'deleted' | 'renamed'
          additions: number
          deletions: number
          isStaged: boolean
        }
      >()

      let totalStaged = 0
      let totalModified = 0

      for (const file of gitDiff.files) {
        gitFileMap.set(file.filename, {
          status: file.status,
          additions: file.additions,
          deletions: file.deletions,
          isStaged: file.isStaged,
        })

        if (file.isStaged) {
          totalStaged++
        }
        if (file.status === 'modified') {
          totalModified++
        }
      }

      // Build file tree with git change information
      const tree = this.buildFileTree(allFiles, gitFileMap, rootPath)

      const gitSummary = {
        totalFiles: gitDiff.files.length,
        totalAdditions: gitDiff.totalAdditions,
        totalDeletions: gitDiff.totalDeletions,
        totalModified,
        totalStaged,
        compareBase: gitDiff.compareBase,
        hasChanges: gitDiff.files.length > 0,
      }

      return {
        tree,
        gitSummary,
      }
    } catch (error) {
      logger.error('Failed to get file tree with changes', error as Error, { taskId })
      throw new WorkspaceError(
        `Failed to get file tree with changes: ${error instanceof Error ? error.message : String(error)}`,
        taskId,
      )
    }
  }

  /**
   * Efficiently get all files in the workspace
   */
  private async getAllFiles(workspacePath: string, rootPath: string): Promise<string[]> {
    const fullPath = path.join(workspacePath, rootPath)
    const allFiles: string[] = []

    async function walkDir(dir: string, relativePath: string): Promise<void> {
      try {
        const entries = await fs.readdir(dir, { withFileTypes: true })

        for (const entry of entries) {
          // Skip hidden files, node_modules, and common ignore patterns
          if (
            entry.name.startsWith('.') ||
            entry.name === 'node_modules' ||
            entry.name === '__pycache__'
          ) {
            continue
          }

          const fullEntryPath = path.join(dir, entry.name)
          const relativeEntryPath = relativePath ? path.join(relativePath, entry.name) : entry.name

          if (entry.isDirectory()) {
            allFiles.push(`${relativeEntryPath}/`)
            await walkDir(fullEntryPath, relativeEntryPath)
          } else {
            allFiles.push(relativeEntryPath)
          }
        }
      } catch (error) {
        // Continue on permission errors or other issues
        logger.debug('Error reading directory', {
          dir,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    await walkDir(fullPath, rootPath === '.' ? '' : rootPath)
    return allFiles.sort()
  }

  /**
   * Build hierarchical file tree with git change information
   */
  private buildFileTree(
    files: string[],
    gitFileMap: Map<
      string,
      {
        status: 'added' | 'modified' | 'deleted' | 'renamed'
        additions: number
        deletions: number
        isStaged: boolean
      }
    >,
    rootPath: string,
  ): FileTreeNode[] {
    const tree: FileTreeNode[] = []
    const nodeMap = new Map<string, FileTreeNode>()

    // Create root node if needed
    if (rootPath !== '.' && rootPath !== '') {
      const rootNode: FileTreeNode = {
        name: path.basename(rootPath),
        path: rootPath,
        type: 'directory',
        isChanged: false,
        children: [],
      }
      tree.push(rootNode)
      nodeMap.set(rootPath, rootNode)
    }

    for (const filePath of files) {
      const isDirectory = filePath.endsWith('/')
      const cleanPath = isDirectory ? filePath.slice(0, -1) : filePath
      const pathParts = cleanPath.split('/').filter((part) => part.length > 0)

      // Build intermediate directories
      let currentPath = rootPath === '.' ? '' : rootPath
      let currentChildren = tree

      for (let i = 0; i < pathParts.length - 1; i++) {
        const part = pathParts[i]
        const partPath = currentPath ? path.join(currentPath, part) : part

        if (!nodeMap.has(partPath)) {
          const dirNode: FileTreeNode = {
            name: part,
            path: partPath,
            type: 'directory',
            isChanged: false,
            children: [],
          }
          nodeMap.set(partPath, dirNode)
          currentChildren.push(dirNode)
        }

        const dirNode = nodeMap.get(partPath)!
        currentChildren = dirNode.children!
        currentPath = partPath
      }

      // Add the file/directory node
      const lastPart = pathParts[pathParts.length - 1]
      const fullPath = currentPath ? path.join(currentPath, lastPart) : lastPart

      if (!nodeMap.has(fullPath)) {
        const gitInfo = gitFileMap.get(cleanPath)
        const node: FileTreeNode = {
          name: lastPart,
          path: fullPath,
          type: isDirectory ? 'directory' : 'file',
          isChanged: !!gitInfo,
          ...(gitInfo && {
            gitStatus: gitInfo.status,
            additions: gitInfo.additions,
            deletions: gitInfo.deletions,
            isStaged: gitInfo.isStaged,
          }),
          ...(isDirectory && { children: [] }),
        }

        nodeMap.set(fullPath, node)
        currentChildren.push(node)

        // Mark parent directories as changed if this file is changed
        if (gitInfo) {
          this.markParentDirectoriesAsChanged(nodeMap, fullPath)
        }
      }
    }

    // Sort children recursively
    this.sortTreeNodes(tree)

    return tree
  }

  /**
   * Mark parent directories as changed when a child file is changed
   */
  private markParentDirectoriesAsChanged(
    nodeMap: Map<string, FileTreeNode>,
    filePath: string,
  ): void {
    const parts = filePath.split('/')

    for (let i = parts.length - 2; i >= 0; i--) {
      const parentPath = parts.slice(0, i + 1).join('/')
      const parentNode = nodeMap.get(parentPath)

      if (parentNode && parentNode.type === 'directory') {
        parentNode.isChanged = true
      }
    }
  }

  /**
   * Sort tree nodes with directories first, then files, alphabetically
   */
  private sortTreeNodes(nodes: FileTreeNode[]): void {
    nodes.sort((a, b) => {
      // Directories first
      if (a.type === 'directory' && b.type === 'file') return -1
      if (a.type === 'file' && b.type === 'directory') return 1

      // Then alphabetically
      return a.name.localeCompare(b.name)
    })

    // Recursively sort children
    for (const node of nodes) {
      if (node.children) {
        this.sortTreeNodes(node.children)
      }
    }
  }
}

export function createWorkspaceService(
  repositoryService: RepositoryService,
  eventService: EventService,
  storage: StorageBackend,
): WorkspaceService {
  return new WorkspaceService(repositoryService, eventService, storage)
}
