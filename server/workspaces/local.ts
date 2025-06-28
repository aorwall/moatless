import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import { WorkspaceError } from '@shared/constants/errors.js'
import type { Environment, LocalRepository } from '@shared/types/environment.js'
import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@shared/types/workspace.js'
import type { BaseAgent } from '../services/agents/base-agent.js'
import type { EventService } from '../services/event.service.js'
import { searchFilesInDirectoryNew } from '../utils/file-search.js'
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

      // Step 3: Check if workspace path exists, recreate if needed
      const repo = this.environment.repository as LocalRepository
      if (repo.useGitWorktrees && !(await this.pathExists(this.workspacePath))) {
        this.logger.warn('Workspace path does not exist, recreating git worktree', {
          workspacePath: this.workspacePath,
        })
        
        // Use the taskId we already have
        await this.recreateGitWorktree(repo.path, this.workspacePath, this.taskId)
      }

      // Step 4: Extract branch info from existing workspace if needed
      if (repo.useGitWorktrees) {
        await this.extractBranchInfoFromExistingWorkspace(repo)
      }

      // Step 5: Mark setup as partially complete (workspace ready)
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

      // Mark setup as failed
      this.setupComplete = false
      this.setupInProgress = false
      
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
          this.logger.error('Setup command failed', undefined, {
            command: trimmedCommand,
            exitCode: result.exitCode,
            output: result.output,
          })
          throw new Error(`Setup command failed with exit code ${result.exitCode}: ${trimmedCommand}\n${result.output}`)
        }
      } catch (error) {
        this.logger.error('Setup command execution failed', error as Error, {
          command: trimmedCommand,
        })
        throw error
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
        // For non-worktree setups, we should NOT remove the workspace directory
        // as it's the actual repository path
        this.logger.debug('Skipping directory removal for non-worktree workspace', {
          workspacePath: this.workspacePath,
          useGitWorktrees: repo.useGitWorktrees,
        })
      }
    } else {
      this.logger.debug('No workspace path to clean up')
    }

    const teardownTime = Date.now() - startTime
    this.logger.info('Workspace teardown completed', { teardownTimeMs: teardownTime })
  }

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        404,
      )
    }

    return await searchFilesInDirectoryNew(this.workspacePath, request)
  }

  async getGitDiff(): Promise<string> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        404,
      )
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

  async hasUnmergedCommits(): Promise<boolean> {
    // If workspace doesn't exist or is not set up, no unmerged commits
    if (
      !this.workspacePath ||
      !(await this.pathExists(this.workspacePath)) ||
      this.getStatus() === WorkspaceStatus.NOT_SETUP
    ) {
      return false
    }

    // Import the function from git-utils
    const { hasUnmergedCommits } = await import('../utils/git-utils.js')
    return hasUnmergedCommits(this.workspacePath, this.originalBranch)
  }

  async isBehindBase(): Promise<boolean> {
    // If workspace doesn't exist or is not set up, not behind
    if (
      !this.workspacePath ||
      !(await this.pathExists(this.workspacePath)) ||
      this.getStatus() === WorkspaceStatus.NOT_SETUP
    ) {
      return false
    }

    // Import the function from git-utils
    const { isBehindBase } = await import('../utils/git-utils.js')
    return isBehindBase(this.workspacePath, this.originalBranch)
  }

  async getGitStatus(): Promise<import('@shared/types/api.js').GitStatus> {
    // If workspace doesn't exist or is not set up, return defaults
    if (
      !this.workspacePath ||
      !(await this.pathExists(this.workspacePath)) ||
      this.getStatus() === WorkspaceStatus.NOT_SETUP
    ) {
      return {
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasUnmergedCommits: false,
        isBehindBase: false,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
      }
    }

    // Import the function from git-utils
    const { getComprehensiveGitStatus } = await import('../utils/git-utils.js')
    return getComprehensiveGitStatus(this.workspacePath, this.originalBranch)
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

    // Security: Ensure path is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: Path outside workspace')
    }

    try {
      const stats = await fs.stat(fullPath)
      if (stats.isFile()) {
        return [relativePath]
      }

      const files: string[] = []

      async function walkDir(dir: string, baseDir: string, basePath: string): Promise<void> {
        const entries = await fs.readdir(dir, { withFileTypes: true })

        for (const entry of entries) {
          const fullPath = path.join(dir, entry.name)
          const relativePath = path.join(basePath, entry.name)

          // Skip hidden files and common ignore patterns
          if (entry.name.startsWith('.') || entry.name === 'node_modules') {
            continue
          }

          if (entry.isDirectory()) {
            files.push(relativePath + '/')
            if (recursive) {
              await walkDir(fullPath, baseDir, relativePath)
            }
          } else {
            files.push(relativePath)
          }
        }
      }

      await walkDir(fullPath, this.workspacePath, relativePath === '.' ? '' : relativePath)
      return files.sort()
    } catch (error) {
      if ((error as any).code === 'ENOENT') {
        throw new Error(`Path not found: ${relativePath}`)
      }
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
      // Check file exists and is readable
      const stats = await fs.stat(fullPath)
      if (!stats.isFile()) {
        throw new Error('Path is not a file')
      }

      // Read file with size limit (10MB)
      const MAX_FILE_SIZE = 10 * 1024 * 1024
      if (stats.size > MAX_FILE_SIZE) {
        throw new Error('File too large to read')
      }

      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      if ((error as any).code === 'ENOENT') {
        throw new Error(`File not found: ${filePath}`)
      }
      if (error instanceof Error && error.message.includes('Access denied')) {
        throw error
      }
      if (error instanceof Error && error.message.includes('File too large')) {
        throw error
      }
      if (error instanceof Error && error.message.includes('Path is not a file')) {
        throw error
      }
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

  private async recreateGitWorktree(
    repoPath: string,
    workspacePath: string,
    taskId: string,
  ): Promise<void> {
    this.logger.debug('Recreating git worktree', {
      repoPath,
      workspacePath,
      taskId,
    })

    try {
      // Determine branch name - use existing branch if we have it, otherwise create new one
      const branchName = this.branchName || this.branch || `workspace-${taskId}`
      
      // Check if branch already exists
      let branchExists = false
      try {
        const { stdout } = await execAsync(`git show-ref --verify refs/heads/${branchName}`, {
          cwd: repoPath,
          timeout: 5000,
        })
        branchExists = !!stdout
      } catch {
        // Branch doesn't exist
      }

      if (branchExists) {
        // Create worktree from existing branch (no -b flag)
        this.logger.debug('Creating worktree from existing branch', { branch: branchName })
        await execAsync(`git worktree add "${workspacePath}" "${branchName}"`, {
          cwd: repoPath,
          timeout: 30000,
        })
      } else {
        // Create new branch for worktree
        this.logger.debug('Creating worktree with new branch', { branch: branchName })
        await execAsync(`git worktree add -b "${branchName}" "${workspacePath}"`, {
          cwd: repoPath,
          timeout: 30000,
        })
      }

      this.branch = branchName
      this.logger.info('Git worktree recreated successfully', {
        workspacePath,
        taskId,
        branchName,
      })
    } catch (error) {
      this.logger.error('Failed to recreate git worktree', error as Error, {
        repoPath,
        workspacePath,
        taskId,
      })
      throw error
    }
  }

  async isWorkspacePathValid(): Promise<boolean> {
    if (!this.workspacePath) return false

    try {
      await fs.access(this.workspacePath)

      // For git worktrees, also check if it's still a valid git repository
      const repo = this.environment.repository as LocalRepository
      if (repo.useGitWorktrees) {
        return await isGitRepository(this.workspacePath)
      }

      return true
    } catch {
      return false
    }
  }
}
