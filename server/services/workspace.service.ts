import { promises as fs } from 'node:fs'
import path from 'node:path'
import { isGitRepository } from '@server/utils/index.js'
import {
  EnvironmentNotFoundError,
  TaskNotFoundError,
  WorkspaceError,
} from '@shared/constants/errors.js'
import type { FastifyInstance } from 'fastify'
import type { CommandResult, LocalRepository, ServerStatus } from '../../shared/types/index.js'
import { WorkspaceStatus } from '../../shared/types/workspace.js'
import { logger } from '../utils/logger.js'
import type { BaseWorkspace, FileSearchRequest, FileSearchResponse } from '../workspaces/base.js'
import { DockerWorkspace } from '../workspaces/docker.js'
import { LocalWorkspace } from '../workspaces/local.js'
import type { EnvironmentService } from './environment.service.js'
import type { EventService } from './event.service.js'
import type { PreviewEnvironmentService } from './preview-environment.service.js'
import type { StorageBackend } from './storage.js'

export class WorkspaceService {
  private activeWorkspaces: Map<string, BaseWorkspace> = new Map()
  private backgroundTasks: Map<string, Set<Promise<unknown>>> = new Map()
  private previewEnvironmentService?: PreviewEnvironmentService

  constructor(
    private environmentService: EnvironmentService,
    private eventService: EventService,
    private storage: StorageBackend,
  ) {}

  async createWorkspace(
    taskId: string,
    environmentId: string,
    branchName?: string,
  ): Promise<BaseWorkspace> {
    const environment = await this.environmentService.getEnvironment(environmentId)
    if (!environment) {
      logger.error('Environment not found', undefined, { environmentId })
      throw new EnvironmentNotFoundError(environmentId)
    }

    // Create workspace path for local repositories
    let workspacePath: string | undefined
    if (environment.repository.type === 'local') {
      workspacePath = await this.createWorkspaceRepository(
        environment.repository,
        taskId,
        branchName,
      )
    }

    let workspace: BaseWorkspace
    if (environment.codeExecution.type === 'docker') {
      workspace = new DockerWorkspace(taskId, environment, this.eventService)
    } else {
      workspace = new LocalWorkspace(
        taskId,
        environment,
        this.eventService,
        undefined, // originalBranch
        branchName,
        workspacePath,
      )
    }

    this.activeWorkspaces.set(taskId, workspace)

    return workspace
  }

  private async createWorkspaceRepository(
    repo: LocalRepository,
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

    // Create worktree if it doesn't exist
    if (!(await this.pathExists(workspacePath))) {
      await this.createGitWorktree(repo.path, workspacePath, taskId, branchName)
    }

    return workspacePath
  }

  private getBranchName(taskId: string): string {
    return `workspace-${taskId}`
  }

  private getWorktreeFolderName(repo: LocalRepository, taskId: string): string {
    // Get the last directory name from the repo path
    const repoLastDirName = path.basename(repo.path)

    // Use the branch name for the worktree folder
    return `${repoLastDirName}-${this.getBranchName(taskId)}`
  }

  private async createGitWorktree(
    repoPath: string,
    workspacePath: string,
    taskId: string,
    branchName?: string,
  ): Promise<void> {
    logger.debug('Creating git worktree', { repoPath, workspacePath, taskId, branchName })

    const { exec } = await import('node:child_process')
    const { promisify } = await import('node:util')
    const execAsync = promisify(exec)

    try {
      if (branchName) {
        // Create worktree from existing branch (no -b flag)
        logger.debug('Using existing branch for worktree', { branch: branchName })
        await execAsync(`git worktree add "${workspacePath}" "${branchName}"`, {
          cwd: repoPath,
          timeout: 30000,
        })
      } else {
        // Create new branch for worktree
        const newBranch = this.getBranchName(taskId)
        logger.debug('Creating new branch for worktree', { branch: newBranch })
        await execAsync(`git worktree add -b "${newBranch}" "${workspacePath}"`, {
          cwd: repoPath,
          timeout: 30000,
        })
      }

      logger.info('Git worktree created successfully', {
        workspacePath,
        taskId,
        branchName: branchName || this.getBranchName(taskId),
      })
    } catch (error) {
      logger.error('Failed to create git worktree', error as Error, {
        repoPath,
        workspacePath,
        taskId,
      })
      throw error
    }
  }

  // Private helper methods
  private getWorktreeDir(repo: LocalRepository): string {
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

  async getOrCreateWorkspace(taskId: string, branchName?: string): Promise<BaseWorkspace> {
    const workspaceLogger = logger.child({ taskId })

    // Check if workspace already exists
    const existing = this.activeWorkspaces.get(taskId)
    if (existing) {
      workspaceLogger.debug(`Workspace already exists, returning existing instance`, {
        status: existing.getStatus(),
      })
      return existing
    }

    // Get task to retrieve environment ID and workspace path
    const task = await this.storage.getTask(taskId)
    if (!task) {
      workspaceLogger.error('Task not found', undefined, { taskId })
      throw new TaskNotFoundError(taskId)
    }

    const environmentId = task.environmentId
    const existingWorkspacePath = task.workspacePath

    workspaceLogger.debug('Retrieved task data', {
      environmentId,
      hasExistingWorkspacePath: !!existingWorkspacePath,
    })

    // Get environment
    const environment = await this.environmentService.getEnvironment(environmentId)
    if (!environment) {
      workspaceLogger.error('Environment not found', undefined, { environmentId })
      throw new EnvironmentNotFoundError(environmentId)
    }

    workspaceLogger.debug('Environment loaded successfully', {
      environmentType: environment.repository.type,
      codeExecutionType: environment.codeExecution.type,
      useGitWorktrees:
        environment.repository.type === 'local' ? environment.repository.useGitWorktrees : false,
    })

    try {
      let workspace: BaseWorkspace
      if (environment.codeExecution.type === 'docker') {
        workspaceLogger.debug('Creating Docker workspace with existing path')
        workspace = new DockerWorkspace(taskId, environment, this.eventService)
      } else {
        workspaceLogger.debug('Creating Local workspace with existing path', {
          branchName,
          existingWorkspacePath,
        })
        workspace = new LocalWorkspace(
          taskId,
          environment,
          this.eventService,
          undefined,
          branchName,
          existingWorkspacePath,
        )
      }

      // Store the workspace in the active workspaces map
      this.activeWorkspaces.set(taskId, workspace)

      return workspace
    } catch (error) {
      workspaceLogger.error('Workspace creation failed', error as Error, {
        environmentType: environment.repository.type,
        codeExecutionType: environment.codeExecution.type,
      })
      throw error
    }
  }

  async getWorkspace(taskId: string): Promise<BaseWorkspace | null> {
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

  async listFiles(
    taskId: string,
    relativePath: string = '.',
    recursive: boolean = false,
  ): Promise<string[]> {
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

  async getGitDiff(taskId: string): Promise<unknown> {
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
      workspaceLogger.debug('Initiating workspace teardown', {
        workspaceType: workspace.type,
        force: true,
      })

      await workspace.teardown(true) // Force cleanup

      const teardownTime = Date.now() - startTime
      workspaceLogger.debug('Workspace teardown completed', { teardownTimeMs: teardownTime })

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
      throw error
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

    // Check if workspace has getGitStatus method (LocalWorkspace)
    let gitStatus:
      | { hasUnmergedCommits?: boolean; isBehindBase?: boolean; hasUncommittedChanges?: boolean }
      | undefined
    if ('getGitStatus' in workspace && typeof workspace.getGitStatus === 'function') {
      gitStatus = await workspace.getGitStatus()
    }

    // For backward compatibility, also get individual values
    let hasUnmergedCommits = gitStatus?.hasUnmergedCommits ?? false
    let isBehindBase = gitStatus?.isBehindBase ?? false

    // If gitStatus is not available, fall back to individual methods
    if (!gitStatus) {
      if ('hasUnmergedCommits' in workspace && typeof workspace.hasUnmergedCommits === 'function') {
        hasUnmergedCommits = await workspace.hasUnmergedCommits()
      }
      if ('isBehindBase' in workspace && typeof workspace.isBehindBase === 'function') {
        isBehindBase = await workspace.isBehindBase()
      }
    }

    return {
      status: workspace.getStatus(),
      hasUncommittedChanges:
        gitStatus?.hasUncommittedChanges ?? (await workspace.hasUncommittedChanges()),
      hasUnmergedCommits,
      isBehindBase,
      gitStatus,
      backgroundTaskCount,
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

  public getBackgroundTaskCount(taskId: string): number {
    return this.backgroundTasks.get(taskId)?.size || 0
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
}

export function createWorkspaceService(
  _app: FastifyInstance,
  environmentService: EnvironmentService,
  eventService: EventService,
  storage: StorageBackend,
): WorkspaceService {
  return new WorkspaceService(environmentService, eventService, storage)
}
