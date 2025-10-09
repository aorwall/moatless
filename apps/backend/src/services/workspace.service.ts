import path from 'node:path'
import type { PreviewServerLifecycleSink } from '@moatless/preview-server'
import type {
  CommandExecutionOptions,
  DeployResponse,
  RepositoryConfig,
  ShellCommandLogEvent,
  WorkspaceStatusResponse,
} from '@moatless/types'
import { RepositoryNotFoundError, TaskNotFoundError, WorkspaceError } from '@moatless/types'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import { DockerEnvironment } from '../environments/docker-environment.js'
import { LocalEnvironment } from '../environments/local-environment.js'
import { getDataDir } from '../utils/data-dir.js'
import { pathExists } from '../utils/filesystem.js'
import {
  branchExists,
  cloneRepository,
  createBranch,
  createGitWorktree,
  isGitRepository,
  isGitWorktree,
} from '../utils/index.js'
import { logger } from '../utils/logger.js'
import { clearTaskLog, getTaskLogPath } from '../utils/task-log.js'
import { Workspace } from '../workspaces/workspace.js'
import type { DockerService } from './docker.service.js'
import type { EventService } from './event.service.js'
import type { RepositoryService } from './repository.service.js'
import { cleanupScreenshotService, getScreenshotService } from './screenshot.service.js'
import type { StorageBackend } from './storage.js'

export class WorkspaceService {
  private activeWorkspaces: Map<string, Workspace> = new Map()
  private backgroundTasks: Map<string, Set<Promise<unknown>>> = new Map()

  constructor(
    private repositoryService: RepositoryService,
    private storage: StorageBackend,
    private eventService: EventService,
    private dockerService: DockerService,
    private claudeService: ClaudeService,
    private previewLifecycleSink: PreviewServerLifecycleSink,
  ) {}

  async createWorkspace(
    taskId: string,
    repositoryId: string,
    branchName?: string,
    newBranch?: string,
  ): Promise<Workspace> {
    logger.info('Starting workspace creation', {
      taskId,
      repositoryId,
      branchName,
      newBranch,
    })

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
    const workspacePath = await this.createWorkspaceRepository(
      repository,
      taskId,
      branchName,
      newBranch,
    )

    logger.info('Workspace repository created', {
      taskId,
      workspacePath,
      branchName,
    })

    // Clear task logs for fresh workspace
    await clearTaskLog(taskId)
    logger.debug('Task logs cleared for new workspace', { taskId })

    // Create the appropriate environment based on repository config
    let environment: DockerEnvironment | LocalEnvironment
    if (repository.codeExecution.type === 'docker') {
      logger.info('Creating Docker environment', { taskId, workspacePath })
      environment = new DockerEnvironment(
        taskId,
        repository,
        workspacePath,
        this.dockerService,
        this.claudeService,
        this.eventService,
      )
    } else {
      logger.info('Creating Local environment', { taskId, workspacePath })
      const logFilePath = getTaskLogPath(taskId)
      environment = new LocalEnvironment(
        taskId,
        repository,
        workspacePath,
        this.eventService,
        logFilePath,
        this.previewLifecycleSink,
      )
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
    newBranch?: string,
  ): Promise<string> {
    if (!(await pathExists(repo.path))) {
      throw new WorkspaceError(`Repository path does not exist: ${repo.path}`, taskId, 400)
    }

    // If clone on new task is enabled, clone the repository to a new directory
    if (repo.cloneOnNewTask) {
      return await this.createClonedRepository(repo, taskId, branchName, newBranch)
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
    await this.ensureWorktreeExists(repo, taskId, workspacePath, branchName, newBranch)

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

  private getCloneFolderName(repo: RepositoryConfig, taskId: string): string {
    // Get the last directory name from the repo path
    const repoLastDirName = path.basename(repo.path)

    // Use clone prefix to distinguish from worktrees
    return `${repoLastDirName}-clone-${taskId}`
  }

  private async createClonedRepository(
    repo: RepositoryConfig,
    taskId: string,
    branchName?: string,
    newBranch?: string,
  ): Promise<string> {
    const workspaceLogger = logger.child({ taskId })

    // Ensure repository has a remote URL for cloning
    if (!repo.remoteUrl) {
      throw new WorkspaceError(
        'cloneOnNewTask requires a remoteUrl to be configured. Cannot clone without remote URL.',
        taskId,
        400,
      )
    }

    // Create clone path in REPO_DIR (separate from worktrees)
    const cloneDir = this.getCloneDir()
    const clonePath = path.join(cloneDir, this.getCloneFolderName(repo, taskId))

    workspaceLogger.info('Creating cloned repository', { clonePath, taskId })

    // Check if clone already exists
    if (await pathExists(clonePath)) {
      workspaceLogger.info('Cloned repository already exists, using existing clone', { clonePath })
      return clonePath
    }

    try {
      // Clone the repository from remote URL using git-utils
      workspaceLogger.info('Cloning repository from remote URL', {
        remoteUrl: repo.remoteUrl,
        destination: clonePath,
        sourceBranch: branchName,
        newBranch,
      })

      await cloneRepository(repo.remoteUrl, clonePath, branchName, {
        timeout: 300000, // 5 minutes timeout
      })

      // After cloning, create a new branch for this task if newBranch is specified
      // or use a generated branch name to ensure each task has its own branch
      const workspaceBranchName = newBranch || this.getBranchName(taskId)

      workspaceLogger.info('Creating task-specific branch after clone', {
        workspaceBranch: workspaceBranchName,
        sourceBranch: branchName || 'default',
      })

      // Create and checkout the new branch in the cloned repository
      await createBranch(clonePath, workspaceBranchName)

      workspaceLogger.info('Repository cloned and task branch created successfully', {
        clonePath,
        workspaceBranch: workspaceBranchName,
      })
      return clonePath
    } catch (error) {
      workspaceLogger.error('Failed to clone repository from remote URL', error as Error, {
        remoteUrl: repo.remoteUrl,
        destination: clonePath,
        branchName,
        newBranch,
      })
      throw new WorkspaceError(
        `Failed to clone repository from remote URL: ${error instanceof Error ? error.message : String(error)}`,
        taskId,
        500,
      )
    }
  }

  // Private helper methods
  private getWorktreeDir(repo: RepositoryConfig): string {
    // If worktrees are disabled, use repository path directly
    if (!repo.useGitWorktrees) {
      return repo.path
    }

    // Check for environment variable first
    const worktreeDir = process.env['WORKTREE_DIR']
    if (worktreeDir) {
      return worktreeDir
    }

    // Default to "../" relative to repo path
    return path.join(repo.path, '..')
  }

  /**
   * Get the directory where cloned repositories should be stored.
   * Uses REPO_DIR environment variable or defaults to data directory.
   */
  private getCloneDir(): string {
    return process.env['REPO_DIR'] || path.join(getDataDir(), 'repos')
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
    newBranch?: string,
  ): Promise<void> {
    const workspaceLogger = logger.child({ taskId })

    // Check if workspace path exists and is a valid git worktree
    const pathExistsResult = await pathExists(workspacePath)
    const isValidWorktree = pathExistsResult && (await isGitWorktree(workspacePath))

    if (isValidWorktree) {
      workspaceLogger.debug('Worktree already exists and is valid', {
        workspacePath,
      })
      return
    }

    if (pathExistsResult && !isValidWorktree) {
      workspaceLogger.warn('Workspace path exists but is not a valid worktree, will recreate', {
        workspacePath,
      })
    } else {
      workspaceLogger.info('Worktree does not exist, creating new one', {
        workspacePath,
      })
    }

    // Get the workspace branch name - use newBranch if provided, otherwise generate one
    const workspaceBranchName = newBranch || this.getBranchName(taskId)

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

    workspaceLogger.info('Worktree created successfully', {
      workspacePath,
      branchName,
    })
  }

  /**
   * Checks if a branch exists in the repository (locally or on remote)
   */
  private async checkBranchExists(repoPath: string, branchName: string): Promise<boolean> {
    try {
      return await branchExists(repoPath, branchName)
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
      if (!task?.iterations || task.iterations.length === 0) {
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
      logger.error('Failed to get last iteration branch', error as Error, {
        taskId,
      })
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
      if (!task?.workspacePath) {
        workspaceLogger.debug('No workspace path found for task, skipping worktree removal')
        return
      }

      const workspacePath = task.workspacePath
      await this.removeWorktreeIfExists(taskId, workspacePath)
    } catch (error) {
      workspaceLogger.error('Failed to remove worktree', error as Error, {
        taskId,
      })
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
      if (!(await pathExists(workspacePath))) {
        workspaceLogger.debug('Worktree path does not exist, skipping removal', { workspacePath })
        return
      }

      // Check if it's a git worktree
      if (!(await isGitWorktree(workspacePath))) {
        workspaceLogger.debug('Path is not a git worktree, skipping removal', {
          workspacePath,
        })
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

      // Handle different types of workspace paths
      const isClonedRepository = repository.cloneOnNewTask
      const isWorktreeRepository = repository.useGitWorktrees && !isClonedRepository

      if (!(isClonedRepository || isWorktreeRepository)) {
        workspaceLogger.debug(
          'Neither cloning nor worktrees enabled for repository, skipping removal',
          {
            repositoryId: repository.id,
            workspacePath,
          },
        )
        return
      }

      if (isClonedRepository) {
        // Remove cloned repository directory
        workspaceLogger.info('Removing cloned repository', { workspacePath })

        const { rm } = await import('node:fs/promises')
        await rm(workspacePath, { recursive: true, force: true })

        workspaceLogger.info('Cloned repository removed successfully', {
          workspacePath,
        })
      } else if (isWorktreeRepository) {
        // Remove the worktree using git command
        workspaceLogger.info('Removing git worktree', { workspacePath })

        const { exec } = await import('node:child_process')
        const { promisify } = await import('node:util')
        const execAsync = promisify(exec)

        await execAsync(`git worktree remove "${workspacePath}" --force`, {
          cwd: repository.path,
          timeout: 10000,
        })

        workspaceLogger.info('Git worktree removed successfully', {
          workspacePath,
        })
      }
    } catch (error) {
      workspaceLogger.error('Failed to remove git worktree', error as Error, {
        workspacePath,
        // Continue with cleanup even if worktree removal fails
      })
    }
  }

  private async validateExistingClonedWorkspace(
    existingPath: string,
    repository: RepositoryConfig,
    taskId: string,
    branchName: string | undefined,
    workspaceLogger: ReturnType<typeof logger.child>,
  ): Promise<string> {
    const pathExistsResult = await pathExists(existingPath)
    if (pathExistsResult) {
      return existingPath
    }

    workspaceLogger.warn('Existing cloned workspace no longer exists, recreating', {
      existingPath,
    })
    return this.createClonedRepository(repository, taskId, branchName, undefined)
  }

  private async validateExistingWorktree(
    existingPath: string,
    repository: RepositoryConfig,
    taskId: string,
    branchName: string | undefined,
    workspaceLogger: ReturnType<typeof logger.child>,
  ): Promise<string> {
    const pathExistsResult = await pathExists(existingPath)
    const isValidWorktree = pathExistsResult && (await isGitWorktree(existingPath))

    if (isValidWorktree) {
      return existingPath
    }

    workspaceLogger.warn('Existing workspace path is not a valid worktree, recreating', {
      existingPath,
    })

    const lastIterationBranch = await this.getLastIterationBranch(taskId)
    const branchToUse = branchName || lastIterationBranch || undefined

    workspaceLogger.debug('Using branch for worktree recreation', {
      providedBranch: branchName,
      lastIterationBranch,
      branchToUse,
    })

    await this.ensureWorktreeExists(repository, taskId, existingPath, branchToUse)
    return existingPath
  }

  private async resolveWorkspacePath(
    repository: RepositoryConfig,
    taskId: string,
    existingWorkspacePath: string | undefined,
    branchName: string | undefined,
    workspaceLogger: ReturnType<typeof logger.child>,
  ): Promise<string> {
    if (!existingWorkspacePath) {
      const newPath = await this.createWorkspaceRepository(
        repository,
        taskId,
        branchName,
        undefined,
      )
      await clearTaskLog(taskId)
      workspaceLogger.debug('Task logs cleared for new workspace')
      return newPath
    }

    if (repository.cloneOnNewTask) {
      return this.validateExistingClonedWorkspace(
        existingWorkspacePath,
        repository,
        taskId,
        branchName,
        workspaceLogger,
      )
    }

    if (repository.useGitWorktrees) {
      return this.validateExistingWorktree(
        existingWorkspacePath,
        repository,
        taskId,
        branchName,
        workspaceLogger,
      )
    }

    const pathExistsResult = await pathExists(existingWorkspacePath)
    if (!pathExistsResult) {
      throw new WorkspaceError(
        `Workspace path does not exist and neither worktrees nor cloning are enabled: ${existingWorkspacePath}`,
        taskId,
        400,
      )
    }

    return existingWorkspacePath
  }

  private createEnvironment(
    repository: RepositoryConfig,
    taskId: string,
    workspacePath: string,
  ): DockerEnvironment | LocalEnvironment {
    if (repository.codeExecution.type === 'docker') {
      return new DockerEnvironment(
        taskId,
        repository,
        workspacePath,
        this.dockerService,
        this.claudeService,
        this.eventService,
      )
    }

    const logFilePath = getTaskLogPath(taskId)
    return new LocalEnvironment(
      taskId,
      repository,
      workspacePath,
      this.eventService,
      logFilePath,
      this.previewLifecycleSink,
    )
  }

  async getOrCreateWorkspace(taskId: string, branchName?: string): Promise<Workspace> {
    const workspaceLogger = logger.child({ taskId })

    const existing = this.activeWorkspaces.get(taskId)
    if (existing) {
      workspaceLogger.debug('Workspace already exists, returning existing instance', {
        status: existing.getStatus(),
      })
      return existing
    }

    const task = await this.storage.getTask(taskId)
    if (!task) {
      workspaceLogger.error('Task not found', undefined, { taskId })
      throw new TaskNotFoundError(taskId)
    }

    const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
    if (!repository) {
      workspaceLogger.error('Repository not found', undefined, {
        repositoryId: task.repositoryId,
      })
      throw new RepositoryNotFoundError(task.repositoryId)
    }

    workspaceLogger.debug('Repository loaded successfully', {
      repositoryPath: repository.path,
      codeExecutionType: repository.codeExecution.type,
      useGitWorktrees: repository.useGitWorktrees,
    })

    try {
      const workspacePath = await this.resolveWorkspacePath(
        repository,
        taskId,
        task.workspacePath,
        branchName,
        workspaceLogger,
      )

      const environment = this.createEnvironment(repository, taskId, workspacePath)
      const workspace = new Workspace(taskId, repository, environment, workspacePath)

      this.activeWorkspaces.set(taskId, workspace)
      await workspace.setup()

      return workspace
    } catch (error) {
      workspaceLogger.error('Workspace creation failed', error as Error, {
        repositoryPath: repository.path,
        codeExecutionType: repository.codeExecution.type,
      })
      throw error
    }
  }

  getWorkspace(taskId: string): Workspace | null {
    // Check active workspaces only
    return this.activeWorkspaces.get(taskId) || null
  }

  async canManageServer(taskId: string): Promise<boolean> {
    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      // Delegate to workspace's canManageServer method
      return await workspace.canManageServer()
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

    await workspace.setup()

    // Auto-start preview server if configured
    const repository = workspace.environmentConfig
    const serverCommands = repository.previewConfig?.serverStartCommands
    if (serverCommands && serverCommands.length > 0) {
      logger.info('Starting preview server after workspace setup', {
        taskId,
        serverCommands,
      })

      workspace.startServer().catch((error) => {
        logger.warn('Failed to auto-start preview server', {
          error: error instanceof Error ? error.message : String(error),
          taskId,
        })
        // Don't fail workspace setup if server fails to start
      })
    }

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

    // Get workspace status from workspace
    const status = await workspace.getWorkspaceStatus(backgroundTaskCount)

    // Set agent type from task if available
    if (task.agentType) {
      status.agentType = task.agentType
    }

    // Check if repository config is outdated
    try {
      const latestRepo = await this.repositoryService.getRepositoryById(workspace.repositoryId)
      if (latestRepo) {
        status.repositoryUpdatedAt = latestRepo.updatedAt.toISOString()
        // Compare repository updatedAt with environment createdAt
        const envCreatedAt = new Date(status.environmentCreatedAt)
        status.configOutdated = latestRepo.updatedAt > envCreatedAt
      }
    } catch (_error) {
      // If we can't fetch the repository, assume config is not outdated
      status.configOutdated = false
    }

    return status
  }

  // Background task tracking methods
  public trackBackgroundTask(taskId: string, task: Promise<unknown>): void {
    if (!this.backgroundTasks.has(taskId)) {
      this.backgroundTasks.set(taskId, new Set())
    }
    const taskSet = this.backgroundTasks.get(taskId)
    if (!taskSet) {
      throw new Error(`Background task set not found for ${taskId}`)
    }
    taskSet.add(task)

    // Clean up when task completes
    task.finally(() => {
      taskSet.delete(task)
      if (taskSet.size === 0) {
        this.backgroundTasks.delete(taskId)
      }
    })
  }

  async capturePreviewScreenshot(
    taskId: string,
    iterationNumber: number,
  ): Promise<{ screenshotPath: string; thumbnailPath: string } | undefined> {
    const workspaceLogger = logger.child({ taskId, iterationNumber })

    try {
      // Check if server can be managed
      if (!(await this.canManageServer(taskId))) {
        workspaceLogger.debug('Preview server not configured for task')
        return undefined
      }

      // Restart server to ensure fresh state
      workspaceLogger.info('Restarting preview server for screenshot capture')
      const workspace = await this.getOrCreateWorkspace(taskId)
      await workspace.stopServer()
      await new Promise((resolve) => setTimeout(resolve, 1000))
      const serverStatus = await workspace.startServer()

      if (serverStatus.status !== 'started' || !serverStatus.url) {
        workspaceLogger.warn('Server failed to start for screenshot')
        return undefined
      }

      // Wait for server to be responsive
      workspaceLogger.debug('Waiting for server to be ready', {
        url: serverStatus.url,
      })
      await this.waitForServerReady(serverStatus.url, 30000) // 30 second timeout

      // Capture screenshot
      workspaceLogger.info('Capturing preview screenshot', {
        url: serverStatus.url,
      })
      const screenshotService = getScreenshotService()
      const screenshotResult = await screenshotService.captureScreenshot(
        serverStatus.url,
        taskId,
        iterationNumber,
      )

      workspaceLogger.info('Preview screenshot captured successfully', {
        screenshotPath: screenshotResult.screenshotPath,
        thumbnailPath: screenshotResult.thumbnailPath,
      })
      return screenshotResult
    } catch (error) {
      workspaceLogger.error('Failed to capture preview screenshot', error as Error)
      return undefined
    }
  }

  private async waitForServerReady(url: string, timeout = 30000): Promise<void> {
    const startTime = Date.now()

    while (Date.now() - startTime < timeout) {
      try {
        const response = await fetch(url, {
          method: 'HEAD',
          signal: AbortSignal.timeout(5000),
        })

        if (response.ok || response.status < 500) {
          // Server is responding
          logger.debug('Server is ready', { url, status: response.status })
          return
        }

        logger.debug('Server responded with error status', {
          url,
          status: response.status,
        })
      } catch (error) {
        // Server not ready yet
        logger.debug('Server not ready yet', {
          url,
          error: error instanceof Error ? error.message : String(error),
        })
      }

      await new Promise((resolve) => setTimeout(resolve, 1000))
    }

    throw new Error(`Server did not become ready within ${timeout}ms`)
  }

  /**
   * Execute commands in a temporary workspace for repository verification
   */
  async *executeCommandsInTemporaryWorkspace(
    repositoryId: string,
    options: CommandExecutionOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    const workspaceLogger = logger.child({
      repositoryId,
      purpose: options.purpose,
    })

    // Generate temporary task ID
    const timestamp = Date.now()
    const tempTaskId = `${options.purpose || 'execute'}-${repositoryId}-${timestamp}`

    workspaceLogger.info('Creating temporary workspace for command execution', {
      tempTaskId,
      commandCount: options.commands.length,
    })

    let workspace: Workspace | undefined

    try {
      // Create temporary workspace
      workspace = await this.createWorkspace(tempTaskId, repositoryId)

      // Setup the workspace
      await workspace.setup()

      workspaceLogger.info('Temporary workspace ready, executing commands', {
        workspacePath: workspace.getWorkspacePath(),
      })

      // Execute commands with streaming
      for await (const event of workspace.executeCommandsWithStream(options)) {
        yield event
      }
    } catch (error) {
      workspaceLogger.error('Failed to execute commands in temporary workspace', error as Error)

      // Emit error event
      yield {
        type: 'system',
        timestamp: new Date().toISOString(),
        text: `Error: ${error instanceof Error ? error.message : String(error)}\n`,
      }
    } finally {
      // Always cleanup temporary workspace
      if (workspace) {
        try {
          await workspace.teardown(true) // Force cleanup
          this.activeWorkspaces.delete(tempTaskId)
          workspaceLogger.info('Temporary workspace cleaned up', {
            tempTaskId,
          })
        } catch (cleanupError) {
          workspaceLogger.error('Failed to cleanup temporary workspace', cleanupError as Error)
        }
      }
    }
  }

  /**
   * Execute commands in an existing task workspace
   */
  async *executeCommandsInTaskWorkspace(
    taskId: string,
    options: CommandExecutionOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    const workspaceLogger = logger.child({ taskId, purpose: options.purpose })

    workspaceLogger.info('Executing commands in task workspace', {
      commandCount: options.commands.length,
    })

    try {
      // Get or create the workspace
      const workspace = await this.getOrCreateWorkspace(taskId)

      // Ensure workspace is ready
      const environment = workspace.getEnvironment()
      await environment.ensureReady()

      workspaceLogger.info('Task workspace ready, executing commands', {
        workspacePath: workspace.getWorkspacePath(),
      })

      // Execute commands with streaming
      for await (const event of workspace.executeCommandsWithStream(options)) {
        yield event
      }
    } catch (error) {
      workspaceLogger.error('Failed to execute commands in task workspace', error as Error)

      // Emit error event
      yield {
        type: 'system',
        timestamp: new Date().toISOString(),
        text: `Error: ${error instanceof Error ? error.message : String(error)}\n`,
      }
    }
  }

  async deployToNetlify(
    taskId: string,
    taskName: string | undefined,
    authToken: string,
    existingSiteId?: string,
  ): Promise<DeployResponse> {
    const workspaceLogger = logger.child({ taskId })

    workspaceLogger.info('Starting build and deploy to Netlify', {
      taskName,
      hasAuthToken: Boolean(authToken),
    })

    try {
      const workspace = await this.getOrCreateWorkspace(taskId)
      if (!workspace) {
        throw new WorkspaceError(`No workspace found for task ${taskId}`, taskId)
      }

      const environment = workspace.getEnvironment()

      // Ensure environment is ready
      await environment.ensureReady()

      // Build the project
      workspaceLogger.info('Building project for deployment')
      const buildResult = await environment.buildProject()

      if (!buildResult.success) {
        return {
          success: false,
          error: 'Build failed',
          logs: buildResult.logs,
          buildDuration: buildResult.duration,
        }
      }

      // Deploy to Netlify
      const siteName = taskName || `moatless-${taskId.substring(0, 8)}`
      workspaceLogger.info('Deploying to Netlify', {
        siteName,
        existingSiteId,
      })
      const deployResult = await environment.deployToNetlify(taskName, authToken, existingSiteId)

      workspaceLogger.info('Deployment completed', {
        success: deployResult.success,
        deployUrl: deployResult.deployUrl,
        siteUrl: deployResult.siteUrl,
      })

      return {
        success: deployResult.success,
        siteId: deployResult.siteId,
        deployUrl: deployResult.deployUrl,
        siteUrl: deployResult.siteUrl,
        siteName,
        deployId: deployResult.deployId,
        buildDuration: buildResult.duration,
        deployDuration: deployResult.duration,
        error: deployResult.success ? undefined : 'Deployment failed',
        logs: `=== Build Output ===\n${buildResult.logs}\n\n=== Deploy Output ===\n${deployResult.logs}`,
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      workspaceLogger.error('Deploy failed', error as Error)

      return {
        success: false,
        error: errorMessage,
        logs: `Deploy failed: ${errorMessage}`,
      }
    }
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

    // Cleanup screenshot service
    try {
      await cleanupScreenshotService()
      workspaceLogger.debug('Screenshot service cleaned up')
    } catch (error) {
      workspaceLogger.error('Failed to cleanup screenshot service', error as Error)
    }

    workspaceLogger.info('Workspace service graceful shutdown completed')
  }
}

export function createWorkspaceService(
  repositoryService: RepositoryService,
  storage: StorageBackend,
  eventService: EventService,
  dockerService: DockerService,
  claudeService: ClaudeService,
  previewLifecycleSink: PreviewServerLifecycleSink,
): WorkspaceService {
  return new WorkspaceService(
    repositoryService,
    storage,
    eventService,
    dockerService,
    claudeService,
    previewLifecycleSink,
  )
}
