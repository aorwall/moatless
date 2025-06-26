import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { Environment, LocalRepository } from '@shared/types/environment.js'
import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@shared/types/workspace.js'
import type { BaseAgent } from '../services/agents/base-agent.js'
import type { EventService } from '../services/event.service.js'
import { searchFilesInDirectory } from '../utils/file-search.js'
import {
  getCurrentBranch,
  getGitDiffString,
  hasUncommittedChanges,
  isGitRepository,
} from '../utils/git-utils.js'
import { createTaskLogger } from '../utils/logger.js'
import { BaseWorkspace, type FileSearchRequest, type FileSearchResponse } from './base.js'

const execAsync = promisify(exec)

export class LocalWorkspace extends BaseWorkspace {
  private _runningAgent?: BaseAgent
  private setupInProgress = false
  private setupComplete = false
  private workspacePath = ''
  private branch?: string
  private originalBranch?: string
  private branchName?: string
  private _createdAt = new Date()
  private _error?: string
  private logger = createTaskLogger(this.taskId)

  get worktreePath(): string {
    return this.workspacePath
  }

  get type(): 'local' {
    return 'local'
  }

  get status(): 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) return 'error'
    if (this.setupInProgress) return 'initializing'
    if (this.setupComplete) return 'ready'
    return 'initializing'
  }

  get createdAt(): Date {
    return this._createdAt
  }

  get error(): string | undefined {
    return this._error
  }

  get runningAgent(): BaseAgent | undefined {
    return this._runningAgent
  }

  constructor(
    taskId: string,
    environment: Environment,
    eventService: EventService,
    originalBranch?: string,
    branchName?: string,
    existingWorkspacePath?: string,
  ) {
    super(taskId, environment, eventService)
    this.originalBranch = originalBranch
    this.branchName = branchName
    this.logger = createTaskLogger(taskId)

    // Use existing workspace path if provided
    if (existingWorkspacePath) {
      this.workspacePath = existingWorkspacePath
    }

    this.logger.debug('LocalWorkspace constructor', {
      environmentId: environment.id,
      environmentType: environment.repository.type,
      codeExecutionType: environment.codeExecution.type,
      hasOriginalBranch: !!originalBranch,
      hasBranchName: !!branchName,
      hasExistingWorkspacePath: !!existingWorkspacePath,
    })
  }

  async setup(): Promise<void> {
    this.setupInProgress = true
    const startTime = Date.now()

    this.logger.info('Starting LocalWorkspace setup', {
      environmentId: this.environment.id,
      workspacePath: this.workspacePath,
    })

    try {
      // Step 1: Validate repository type
      if (this.environment.repository.type !== 'local') {
        this.logger.error('Invalid repository type for LocalWorkspace', undefined, {
          repositoryType: this.environment.repository.type,
        })
        throw new Error('LocalWorkspace requires local repository')
      }

      // Step 2: Validate workspace path is set
      if (!this.workspacePath) {
        throw new Error('Workspace path must be provided to LocalWorkspace constructor')
      }

      // Step 3: Extract branch info from existing workspace if needed
      const repo = this.environment.repository as LocalRepository
      if (repo.useGitWorktrees) {
        await this.extractBranchInfoFromExistingWorkspace(repo)
      }

      // Step 4: Mark setup as partially complete (workspace ready)
      this.logger.info('Workspace ready, continuing with setup commands in background', {
        workspacePath: this.workspacePath,
      })

      // Step 5: Run setup commands in background
      this.runSetupCommandsInBackground(startTime).catch((error) => {
        this._error = error instanceof Error ? error.message : String(error)
        this.logger.error('Background setup commands failed', error as Error)
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      this._error = error instanceof Error ? error.message : String(error)

      this.logger.error('LocalWorkspace setup failed', error as Error, {
        setupTimeMs: failureTime,
        workspacePath: this.workspacePath,
        setupPhase: 'initialization',
      })

      this.setupInProgress = false
      throw error
    }
  }

  private async extractBranchInfoFromExistingWorkspace(repo: LocalRepository): Promise<void> {
    // If we have an existing workspace path with git worktrees, extract the branch info
    if (repo.useGitWorktrees && (await isGitRepository(this.workspacePath))) {
      try {
        this.branch = await getCurrentBranch(this.workspacePath)
        this.logger.debug('Detected existing worktree branch', { branch: this.branch })
      } catch (error) {
        this.logger.warn('Could not detect branch from existing worktree', { error })
      }
    }
  }

  private async runSetupCommandsInBackground(startTime: number): Promise<void> {
    try {
      // Run setup commands if specified
      await this.runSetupCommands()

      this.setupComplete = true

      const totalSetupTime = Date.now() - startTime
      this.logger.info('LocalWorkspace setup completed successfully', {
        setupTimeMs: totalSetupTime,
        workspacePath: this.workspacePath,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      this._error = error instanceof Error ? error.message : String(error)

      this.logger.error('Setup commands failed', error as Error, {
        setupTimeMs: failureTime,
        workspacePath: this.workspacePath,
      })

      throw error
    } finally {
      this.setupInProgress = false
    }
  }

  private async runSetupCommands(): Promise<void> {
    if (!this.environment.codeExecution.setupCommands) {
      this.logger.debug('No setup commands configured')
      return
    }

    const commands = this.environment.codeExecution.setupCommands.split('\n')
    this.logger.info('Running setup commands', {
      commandCount: commands.filter((c) => c.trim()).length,
    })

    for (const command of commands) {
      const trimmedCommand = command.trim()
      if (!trimmedCommand) continue

      this.logger.debug('Executing setup command', { command: trimmedCommand })

      try {
        const result = await this.runCommand(trimmedCommand)

        if (result.exitCode === 0) {
          this.logger.info('Setup command completed successfully', {
            command: trimmedCommand,
            durationMs: result.durationMs,
            outputLength: result.output.length,
          })
        } else {
          this.logger.warn('Setup command failed', {
            command: trimmedCommand,
            exitCode: result.exitCode,
            outputLength: result.output.length,
          })
        }
      } catch (error) {
        this.logger.error('Setup command execution failed', error as Error, {
          command: trimmedCommand,
        })
      }
    }
  }

  async teardown(force = false): Promise<void> {
    const startTime = Date.now()

    this.logger.info('Starting workspace teardown', {
      force,
      workspacePath: this.workspacePath,
      hasRunningAgent: !!this.runningAgent,
    })

    // Check for uncommitted changes first
    if (!force) {
      this.logger.debug('Checking for uncommitted changes')
      const hasChanges = await this.hasUncommittedChanges()
      if (hasChanges) {
        this.logger.warn('Teardown aborted due to uncommitted changes')
        throw new Error('Workspace has uncommitted changes. Use force=true to close anyway.')
      }
      this.logger.debug('No uncommitted changes detected')
    }

    // Stop the running agent if any
    if (this._runningAgent) {
      this.logger.debug('Stopping running agent')
      try {
        await this._runningAgent.stop()
        this.logger.debug('Agent stopped successfully')
      } catch (error) {
        this.logger.error('Error stopping agent', error as Error)
      }
      this._runningAgent = undefined
    }

    // Reset setup state
    this.setupComplete = false

    // If we created a git worktree, remove it
    if (this.workspacePath && (await this.pathExists(this.workspacePath))) {
      const repo = this.environment.repository as LocalRepository
      if (repo.useGitWorktrees && (this.originalBranch || this.branch)) {
        this.logger.debug('Removing git worktree', {
          repositoryPath: repo.path,
          workspacePath: this.workspacePath,
          branch: this.branch,
        })

        try {
          await this.removeGitWorktree(repo.path, this.workspacePath)
          this.logger.debug('Git worktree removed successfully')
        } catch (error) {
          this.logger.error(
            'Error removing git worktree, attempting manual cleanup',
            error as Error,
          )
          // Try manual cleanup as fallback
          await fs.rm(this.workspacePath, { recursive: true, force: true })
          this.logger.debug('Manual cleanup completed')
        }
      } else {
        this.logger.debug('Removing workspace directory')
        // Just remove the directory
        await fs.rm(this.workspacePath, { recursive: true, force: true })
        this.logger.debug('Workspace directory removed successfully')
      }
    } else {
      this.logger.debug('No workspace path to clean up')
    }

    const teardownTime = Date.now() - startTime
    this.logger.info('Workspace teardown completed', { teardownTimeMs: teardownTime })
  }

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
    }

    return await searchFilesInDirectory(this.workspacePath, request)
  }

  async getGitDiff(): Promise<string> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
    }

    const [diffString] = await getGitDiffString(this.workspacePath, this.originalBranch)
    return diffString
  }

  async runCommand(command: string, timeout?: number): Promise<CommandResult> {
    const startTime = Date.now()

    try {
      const { stdout, stderr } = await execAsync(command, {
        cwd: this.workspacePath,
        timeout: timeout || 120000, // Default 2 minutes
      })

      // Combine stdout and stderr
      let output = stdout
      if (stderr) {
        output += output ? '\n' + stderr : stderr
      }

      return {
        command,
        output,
        exitCode: 0,
        durationMs: Date.now() - startTime,
      }
    } catch (error: any) {
      const output = error.stdout || ''
      const stderr = error.stderr || ''
      const combinedOutput = output + (stderr ? '\n' + stderr : '')

      return {
        command,
        output: combinedOutput || error.message,
        exitCode: error.code || -1,
        durationMs: Date.now() - startTime,
      }
    }
  }

  isRunning(): boolean {
    return this._runningAgent !== undefined
  }

  getStatus(): WorkspaceStatus {
    // Check if agent is running
    if (this._runningAgent) {
      return WorkspaceStatus.RUNNING
    }

    // Check if setup is in progress
    if (this.setupInProgress) {
      return WorkspaceStatus.SETTING_UP
    }

    // Check if workspace directory exists
    if (this.workspacePath) {
      return WorkspaceStatus.IDLE
    }

    // Otherwise, workspace is not set up
    return WorkspaceStatus.NOT_SETUP
  }

  async hasUncommittedChanges(): Promise<boolean> {
    // If workspace doesn't exist or is not set up, no changes
    if (
      !this.workspacePath ||
      !(await this.pathExists(this.workspacePath)) ||
      this.getStatus() === WorkspaceStatus.NOT_SETUP
    ) {
      return false
    }

    return hasUncommittedChanges(this.workspacePath)
  }

  async mergeChanges(): Promise<string> {
    // Check if repository is local
    if (this.environment.repository.type !== 'local') {
      throw new Error('Merge only supported for local repositories')
    }

    const repo = this.environment.repository as LocalRepository

    // Check if worktrees are enabled
    if (!repo.useGitWorktrees) {
      throw new Error('Merge only supported for git worktree workspaces')
    }

    // Check for uncommitted changes
    if (await this.hasUncommittedChanges()) {
      throw new Error('Cannot merge with uncommitted changes')
    }

    // TODO: Implement merge logic
    throw new Error('Merge not yet implemented')
  }

  async stopAgent(): Promise<boolean> {
    if (!this._runningAgent) {
      this.logger.debug('No agent running to stop')
      return false
    }

    this.logger.info('Stopping running agent')
    try {
      await this._runningAgent.stop()
      this._runningAgent = undefined
      this.logger.info('Agent stopped successfully')
      return true
    } catch (error) {
      this.logger.error('Error stopping agent', error as Error)
      this._runningAgent = undefined
      return false
    }
  }

  async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
    const fullPath = path.join(this.workspacePath, relativePath)

    try {
      const stats = await fs.stat(fullPath)
      if (stats.isFile()) {
        return [relativePath]
      }

      const entries = await fs.readdir(fullPath, { withFileTypes: true })
      const files: string[] = []

      for (const entry of entries) {
        const entryPath = path.join(relativePath, entry.name)

        if (entry.isFile()) {
          files.push(entryPath)
        } else if (entry.isDirectory() && recursive && !entry.name.startsWith('.')) {
          const subFiles = await this.listFiles(entryPath, recursive)
          files.push(...subFiles)
        }
      }

      return files.sort()
    } catch (error) {
      throw new Error(
        `Failed to list files: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  async readFile(filePath: string): Promise<string> {
    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)

    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      throw new Error(
        `Failed to read file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  async writeFile(filePath: string, content: string): Promise<void> {
    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)

    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content, 'utf-8')
    } catch (error) {
      throw new Error(
        `Failed to write file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  // Server methods - deprecated, now handled by PreviewEnvironmentService
  async startServer(): Promise<any> {
    throw new Error('Server operations are now handled by PreviewEnvironmentService')
  }

  async stopServer(): Promise<boolean> {
    throw new Error('Server operations are now handled by PreviewEnvironmentService')
  }

  async getServerStatus(): Promise<ServerStatus> {
    throw new Error('Server operations are now handled by PreviewEnvironmentService')
  }

  async getServerLogs(_lines = 100): Promise<string> {
    throw new Error('Server operations are now handled by PreviewEnvironmentService')
  }

  toWorkspaceInfo(): Workspace {
    return {
      id: this.taskId,
      type: 'local' as const,
      environmentId: this.environment.id,
      status: this.status,
      createdAt: this.createdAt,
      path: this.workspacePath,
      error: this.error,
    }
  }

  getWorkspacePath(): string {
    return this.workspacePath
  }

  isWorkspacePathSet(): boolean {
    return !!this.workspacePath
  }

  private async removeGitWorktree(repoPath: string, workspacePath: string): Promise<void> {
    this.logger.debug('Removing git worktree', {
      repoPath,
      workspacePath,
      branch: this.branch,
      willDeleteBranch: !this.branchName && !!this.branch,
    })

    try {
      // Remove worktree - always use --force to ensure removal
      await execAsync(`git worktree remove --force "${workspacePath}"`, {
        cwd: repoPath,
        timeout: 10000,
      })

      this.logger.debug('Worktree removed successfully')

      // Only try to delete the branch if we created it (no custom branch_name provided)
      if (!this.branchName && this.branch) {
        this.logger.debug('Attempting to delete created branch', { branch: this.branch })
        try {
          await execAsync(`git branch -D "${this.branch}"`, {
            cwd: repoPath,
            timeout: 5000,
          })
          this.logger.debug('Branch deleted successfully', { branch: this.branch })
        } catch (error) {
          this.logger.warn('Branch deletion failed (best effort)', {
            branch: this.branch,
            error: error instanceof Error ? error.message : String(error),
          })
          // Branch deletion is best effort
        }
      } else {
        this.logger.debug('Skipping branch deletion (using existing branch or no branch)')
      }
    } catch (error) {
      this.logger.error('Failed to remove worktree', error as Error, { repoPath, workspacePath })
      throw error
    }
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}
