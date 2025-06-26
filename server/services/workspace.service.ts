import type { BaseAgent } from '@server/services/agents/base-agent.js'
import { isGitRepository } from '@server/utils/index.js'
import {
  CriticalError,
  EnvironmentNotFoundError,
  TaskNotFoundError,
  WorkspaceError,
} from '@shared/constants/errors.js'
import type { FastifyInstance } from 'fastify'
import { promises as fs } from 'fs'
import path from 'node:path'
import type {
  CommandResult,
  LocalRepository,
  ServerStatus,
  TaskIteration,
} from '../../shared/types/index.js'
import { WorkspaceStatus } from '../../shared/types/workspace.js'
import { ClaudeCodeAgent } from '../services/agents/claude-code-agent.js'
import { logger } from '../utils/logger.js'
import type { BaseWorkspace, FileSearchRequest, FileSearchResponse } from '../workspaces/base.js'
import { DockerWorkspace } from '../workspaces/docker.js'
import { LocalWorkspace } from '../workspaces/local.js'
import type { EnvironmentService } from './environment.service.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'

export class WorkspaceService {
  private activeWorkspaces: Map<string, BaseWorkspace> = new Map()
  private backgroundTasks: Map<string, Set<Promise<any>>> = new Map()

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
      if (environment.codeExecution.type === 'docker') {
        workspaceLogger.debug('Creating Docker workspace with existing path')
        return new DockerWorkspace(taskId, environment, this.eventService)
      } else {
        workspaceLogger.debug('Creating Local workspace with existing path', {
          branchName,
          existingWorkspacePath,
        })
        return new LocalWorkspace(
          taskId,
          environment,
          this.eventService,
          undefined,
          branchName,
          existingWorkspacePath,
        )
      }
    
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
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.runCommand(command, timeout)
  }

  async getWorkspaceStatus(taskId: string): Promise<any> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    const backgroundTaskCount = this.backgroundTasks.get(taskId)?.size || 0

    return {
      status: workspace.getStatus(),
      hasUncommittedChanges: await workspace.hasUncommittedChanges(),
      backgroundTaskCount,
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

  async runAgent(taskId: string, message: string, iteration: TaskIteration, agentType: string = 'claude-code'): Promise<void> {
    const workspace = await this.getOrCreateWorkspace(taskId)

    // Validate workspace state
    const status = workspace.getStatus()
    if (status === WorkspaceStatus.SETTING_UP) {
      throw new CriticalError('Workspace is still setting up', taskId, iteration)
    }
    if (status === WorkspaceStatus.RUNNING) {
      throw new CriticalError('Agent is already running', taskId, iteration)
    }

    // Create agent
    const agent = await this.createAgent(agentType, taskId)
    
    // Execute agent in background
    const backgroundTask = this.executeAgent(agent, message, taskId, iteration, workspace)
      .catch((error) => {
        logger.error('Background agent execution failed', error as Error, { taskId })
      })
    
    // Track the background task
    this.trackBackgroundTask(taskId, backgroundTask)
    
    // Don't await - let it run in background
    logger.info('Agent execution started in background', { taskId, iterationNumber: iteration.iterationNumber })
  }

  private async createAgent(agentType: string, taskId: string): Promise<BaseAgent> {
    if (agentType === 'claude-code') {
      // TODO: Get claude options from storage/config
      const options = {} // Placeholder for claude options
      return new ClaudeCodeAgent(taskId, options)
    } else {
      throw new Error(`Unknown agent type: ${agentType}`)
    }
  }

  private async executeAgent(
    agent: BaseAgent,
    message: string,
    taskId: string,
    iteration: TaskIteration,
    workspace: BaseWorkspace,
  ): Promise<void> {
    try {
      const runOptions = {
        workspaceDir: workspace.getWorkspacePath(),
        iterationNumber: iteration.iterationNumber,
      }

      // Process agent events
      for await (const event of agent.run(message, runOptions)) {
        // Capture session ID if it's not already set
        if (
          !iteration.agentSessionId &&
          agent.getSessionId()
        ) {
          iteration.agentSessionId = agent.getSessionId()

          // Update the session ID in the task's iteration
          const task = await this.storage.getTask(taskId)
          if (task) {
            const iterationIndex = task.iterations.findIndex(
              (it) => it.iterationNumber === iteration.iterationNumber,
            )
            if (iterationIndex >= 0) {
              task.iterations[iterationIndex].agentSessionId = iteration.agentSessionId
              await this.storage.saveTask(task)
            }
          }
        } else if (event.payload?.type === 'result') {
          iteration.costUsd = event.payload.total_cost_usd ?? 0
          iteration.result = event.payload.result ?? 'No result found in Claude Code SDK result message'

          // Update iteration status to completed after successful execution
          iteration.status = 'completed'
          iteration.endTime = new Date()
          iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()

          // Find and update the iteration in the task's iterations array
          const task = await this.storage.getTask(taskId)
          if (task) {
            const iterationIndex = task.iterations.findIndex(
              (it) => it.iterationNumber === iteration.iterationNumber,
            )
            if (iterationIndex >= 0) {
              await this.storage.saveTask(task)
            }
          }
        }

        // Only emit agent events
        await this.eventService.emitEvent(event)
      }

    } catch (error) {
      // Save error to iteration
      iteration.status = 'failed'
      iteration.error = {
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
        timestamp: new Date(),
      }

      // Find and update the iteration in the task's iterations array
      const task = await this.storage.getTask(taskId)
      if (task) {
        const iterationIndex = task.iterations.findIndex(
          (it) => it.iterationNumber === iteration.iterationNumber,
        )
        if (iterationIndex >= 0) {
          task.iterations[iterationIndex].status = iteration.status
          task.iterations[iterationIndex].error = iteration.error
          await this.storage.saveTask(task)
        }
      }

      // Re-throw critical errors
      if (error instanceof CriticalError) {
        throw error
      }

      // Log non-critical errors
      logger.error('Agent execution error', error as Error)
    }
  }

  // Background task tracking methods
  public trackBackgroundTask(taskId: string, task: Promise<any>): void {
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
    const workspace = await this.getOrCreateWorkspace(taskId)
    return workspace.searchFiles(request)
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
  storage: StorageBackend,
): WorkspaceService {
  return new WorkspaceService(environmentService, eventService, storage)
}
