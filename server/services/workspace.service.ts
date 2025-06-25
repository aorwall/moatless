import {
  EnvironmentNotFoundError,
  TaskNotFoundError,
  WorkspaceError,
} from '@shared/constants/errors.js'
import type { FastifyInstance } from 'fastify'
import type {CommandResult, LocalRepository, ServerStatus} from '../../shared/types/index.js'
import { logger } from '../utils/logger.js'
import type { BaseWorkspace } from '../workspaces/base.js'
import { DockerWorkspace } from '../workspaces/docker.js'
import { LocalWorkspace } from '../workspaces/local.js'
import type { EnvironmentService } from './environment.service.js'
import type { EventService } from './event.service.js'
import type { TaskStorageService } from './task-storage.service.js'
import path from "node:path";
import {isGitRepository} from "@server/utils/index.js";
import {promises as fs} from "fs";
import {BaseAgent} from "@server/services/agents/base-agent.js";

export class WorkspaceService {
  private activeWorkspaces: Map<string, BaseWorkspace> = new Map()

  constructor(
    private environmentService: EnvironmentService,
    private eventService: EventService,
    private taskStorageService: TaskStorageService,
  ) {}

  async createWorkspace(taskId: string, environmentId: string, branchName?: string): Promise<BaseWorkspace> {
    const environment = await this.environmentService.getEnvironment(environmentId)
    if (!environment) {
      logger.error('Environment not found', undefined, { environmentId })
      throw new EnvironmentNotFoundError(environmentId)
    }

    // Create workspace path for local repositories
    let workspacePath: string | undefined
    if (environment.repository.type === 'local') {
      workspacePath = await this.createWorkspaceRepository(environment.repository, taskId, branchName)
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
          workspacePath
      )
    }

    this.activeWorkspaces.set(taskId, workspace)

    return workspace
  }

  private async createWorkspaceRepository(repo: LocalRepository, taskId: string, branchName?: string): Promise<string> {
    if (!(await this.pathExists(repo.path))) {
      throw new Error(`Repository path does not exist: ${repo.path}`)
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
      throw new Error('Repository path is not a git repository')
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


  private async createGitWorktree(repoPath: string, workspacePath: string, taskId: string, branchName?: string): Promise<void> {
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
      logger.error('Failed to create git worktree', error as Error, { repoPath, workspacePath, taskId })
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
    const taskData = await this.taskStorageService.getTaskWithEnvironmentId(taskId)
    if (!taskData) {
      workspaceLogger.error('Task not found', undefined, { taskId })
      throw new TaskNotFoundError(taskId)
    }

    const { task, environmentId } = taskData
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

    // Create workspace using the dedicated createWorkspace method
    const startTime = Date.now()
    let workspace: BaseWorkspace

    try {
      // If we have an existing workspace path, we need to create the workspace differently
      if (existingWorkspacePath) {
        // Create workspace instance directly with existing path
        if (environment.codeExecution.type === 'docker') {
          workspaceLogger.debug('Creating Docker workspace with existing path')
          workspace = new DockerWorkspace(taskId, environment, this.eventService)
        } else {
          workspaceLogger.debug('Creating Local workspace with existing path', { branchName, existingWorkspacePath })
          workspace = new LocalWorkspace(
            taskId,
            environment,
            this.eventService,
            undefined,
            branchName,
            existingWorkspacePath,
          )
        }
        this.activeWorkspaces.set(taskId, workspace)
      } else {
        // Create new workspace (will handle path creation)
        workspaceLogger.debug('Creating new workspace', { branchName })
        workspace = await this.createWorkspace(taskId, environmentId, branchName)
      }

      const creationTime = Date.now() - startTime
      workspaceLogger.debug('Workspace instance created successfully', {
        workspaceType: workspace.type,
        creationTimeMs: creationTime,
      })

      workspaceLogger.info('Workspace creation completed successfully', {
        workspaceType: workspace.type,
        totalTimeMs: Date.now() - startTime,
        status: workspace.getStatus(),
      })

      return workspace
    } catch (error) {
      const failureTime = Date.now() - startTime
      workspaceLogger.error('Workspace creation failed', error as Error, {
        timeToFailureMs: failureTime,
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

  async listFiles(
    taskId: string,
    relativePath: string = '.',
    recursive: boolean = false,
  ): Promise<string[]> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.listFiles(relativePath, recursive)
  }

  async readFile(taskId: string, filePath: string): Promise<string> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.readFile(filePath)
  }

  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    await workspace.writeFile(filePath, content)
  }

  async getGitDiff(taskId: string): Promise<any> {
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
      const task = await this.taskStorageService.getTask(taskId)
      if (task && !task.workspacePath) {
        task.workspacePath = workspacePath
        await this.taskStorageService.saveTask(task)
        logger.debug('Saved workspace path to task', { taskId, workspacePath })
      }
    } else {
      logger.warn('Workspace path not available after setup', { taskId })
    }
  }

  async triggerAgent(
    taskId: string,
    agentType: string,
    message: string,
    iterationNumber: number,
  ): Promise<string | undefined> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    const agent = await workspace.triggerAgent(agentType, message, iterationNumber)
    return agent.getSessionId()
  }

  async startWorkspace(
    taskId: string,
    agentType: string,
    message: string,
    iterationNumber: number,
  ): Promise<BaseAgent> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    return await workspace.start(agentType, message, iterationNumber)
  }

  async runCommand(taskId: string, command: string, timeout?: number): Promise<CommandResult> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.runCommand(command, timeout)
  }

  async getWorkspaceStatus(taskId: string): Promise<any> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    return {
      status: workspace.getStatus(),
      hasUncommittedChanges: await workspace.hasUncommittedChanges(),
    }
  }

  async startServer(taskId: string): Promise<any> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    if (!workspace.startServer) {
      throw new WorkspaceError(`Server not supported for this workspace type`, taskId)
    }

    return workspace.startServer()
  }

  async stopServer(taskId: string): Promise<boolean> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    if (!workspace.stopServer) {
      return false
    }

    return workspace.stopServer()
  }

  async getServerStatus(taskId: string): Promise<ServerStatus> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    if (!workspace.getServerStatus) {
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }

    return workspace.getServerStatus()
  }

  async getServerLogs(taskId: string, lines?: number): Promise<string> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    if (!workspace.getServerLogs) {
      return 'Server logs not available for this workspace type'
    }

    return workspace.getServerLogs(lines)
  }
}

export function createWorkspaceService(
  _app: FastifyInstance,
  environmentService: EnvironmentService,
  eventService: EventService,
  taskStorageService: TaskStorageService,
): WorkspaceService {
  return new WorkspaceService(environmentService, eventService, taskStorageService)
}
