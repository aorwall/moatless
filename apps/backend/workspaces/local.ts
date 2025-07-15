import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import { WorkspaceError } from '@moatless/types'
import type { GitChanges, GitStatus } from '@moatless/types'
import type { RepositoryConfig } from '@moatless/types'

import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@moatless/types'
import { LocalEnvironment } from '../environments/local-environment.js'
import type { BaseAgent, EventService } from '../services/index.js'
import {
  type GitDiffResponse,
  getCurrentBranch,
  getGitDiffDetailed,
  hasUncommittedChanges,
  isGitRepository,
  searchFilesInDirectoryNew,
} from '../utils/index.js'
import { createTaskLogger } from '../utils/logger.js'
import { BaseWorkspace, type FileSearchRequest, type FileSearchResponse } from './base.js'

const execAsync = promisify(exec)

export class LocalWorkspace extends BaseWorkspace {
  private environment: LocalEnvironment
  private workspacePath = ''
  private branch?: string
  private originalBranch?: string
  private branchName?: string
  private logger = createTaskLogger(this.taskId)

  get type(): 'local' {
    return 'local'
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

  get runningAgent(): BaseAgent | undefined {
    return this.environment.isRunning() ? this.environment.getRunningAgent() : undefined
  }

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    eventService: EventService,
    originalBranch?: string,
    branchName?: string,
    existingWorkspacePath?: string,
  ) {
    super(taskId, repositoryConfig, eventService)
    this.originalBranch = originalBranch
    this.branchName = branchName
    this.logger = createTaskLogger(taskId)

    // Use existing workspace path if provided
    if (existingWorkspacePath) {
      this.workspacePath = existingWorkspacePath

      // For existing workspaces, immediately try to detect branch info
      if (repositoryConfig.useGitWorktrees) {
        this.detectBranchInfoSync(existingWorkspacePath)
      }
    }

    // Initialize environment
    this.environment = new LocalEnvironment(
      taskId,
      repositoryConfig,
      eventService,
      this.workspacePath,
    )

    this.logger.debug('LocalWorkspace constructor', {
      repositoryId: repositoryConfig.id,
      repositoryPath: repositoryConfig.path,
      codeExecutionType: repositoryConfig.codeExecution.type,
      hasOriginalBranch: !!originalBranch,
      hasBranchName: !!branchName,
      hasExistingWorkspacePath: !!existingWorkspacePath,
      detectedBranch: this.branch,
    })
  }

  private detectBranchInfoSync(workspacePath: string): void {
    // Try to extract branch info from workspace path pattern
    // Pattern: <repo-name>-workspace-<task-id>
    if (workspacePath.includes('-workspace-')) {
      const match = workspacePath.match(/workspace-([a-f0-9-]+)$/)
      if (match) {
        const extractedTaskId = match[1]
        // Set the branch based on the expected pattern
        this.branch = `workspace-${extractedTaskId}`
        // Since this appears to be a created workspace, set original branch
        this.originalBranch = this.originalBranch || 'main'
        this.logger.debug('Detected branch info from workspace path', {
          workspacePath,
          branch: this.branch,
          originalBranch: this.originalBranch,
        })
      }
    }
  }

  private async detectBranchFromWorktree(): Promise<string | undefined> {
    try {
      return await getCurrentBranch(this.workspacePath)
    } catch {
      return undefined
    }
  }

  async setup(): Promise<void> {
    this.logger.info('Starting LocalWorkspace setup', {
      repositoryId: this.repositoryConfig.id,
      workspacePath: this.workspacePath,
    })

    try {
      // Step 1: Validate workspace path is set
      if (!this.workspacePath) {
        throw new Error('Workspace path must be provided to LocalWorkspace constructor')
      }

      // Step 2: Check if workspace path exists, recreate if needed
      if (this.repositoryConfig.useGitWorktrees && !(await this.pathExists(this.workspacePath))) {
        this.logger.warn('Workspace path does not exist, recreating git worktree', {
          workspacePath: this.workspacePath,
        })

        // Use the taskId we already have
        await this.recreateGitWorktree(this.repositoryConfig.path, this.workspacePath, this.taskId)
      }

      // Step 3: Extract branch info from existing workspace if needed
      if (this.repositoryConfig.useGitWorktrees) {
        await this.extractBranchInfoFromExistingWorkspace(this.repositoryConfig)
      }

      // Update environment with the resolved workspace path
      this.environment = new LocalEnvironment(
        this.taskId,
        this.repositoryConfig,
        this.eventService,
        this.workspacePath,
      )

      // Step 4: Setup the environment (will run setup commands)
      await this.environment.setup()

      this.logger.info('LocalWorkspace setup completed', {
        workspacePath: this.workspacePath,
      })
    } catch (error) {
      this.logger.error('LocalWorkspace setup failed', error as Error, {
        workspacePath: this.workspacePath,
      })
      throw error
    }
  }

  private async extractBranchInfoFromExistingWorkspace(repo: RepositoryConfig): Promise<void> {
    // If we have an existing workspace path with git worktrees, extract the branch info
    if (repo.useGitWorktrees && (await isGitRepository(this.workspacePath))) {
      try {
        this.branch = await getCurrentBranch(this.workspacePath)
        this.logger.debug('Detected existing worktree branch', { branch: this.branch })

        // Also try to detect original branch from worktree name if not set
        if (!this.originalBranch && this.workspacePath.includes('-workspace-')) {
          // Try to extract task ID from workspace path to reconstruct branch name
          const match = this.workspacePath.match(/workspace-([a-f0-9-]+)$/)
          if (match) {
            const extractedTaskId = match[1]
            const reconstructedBranch = `workspace-${extractedTaskId}`
            if (this.branch === reconstructedBranch) {
              // This appears to be a created branch, not original
              this.originalBranch = 'main' // Default to main as original
              this.logger.debug('Detected this is a created workspace branch', {
                branch: this.branch,
                originalBranch: this.originalBranch,
              })
            }
          }
        }
      } catch (error) {
        this.logger.warn('Could not detect branch from existing worktree', { error })
      }
    }
  }

  // Removed unused methods

  async teardown(force = false): Promise<void> {
    const startTime = Date.now()

    this.logger.info('Starting workspace teardown', {
      force,
      workspacePath: this.workspacePath,
      hasRunningAgent: !!this.runningAgent,
      useGitWorktrees: this.repositoryConfig.useGitWorktrees,
      branch: this.branch,
      originalBranch: this.originalBranch,
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

    // Teardown the environment (will stop agent and server)
    await this.environment.teardown(force)

    // If we have a workspace path and worktrees are enabled, check if it needs cleanup
    if (this.workspacePath && this.repositoryConfig.useGitWorktrees) {
      try {
        // Check if this is actually a git worktree
        const { isGitWorktree } = await import('../utils/git-utils.js')
        const isWorktree = await isGitWorktree(this.workspacePath)

        if (isWorktree) {
          this.logger.info('Removing git worktree', {
            workspacePath: this.workspacePath,
            repositoryPath: this.repositoryConfig.path,
          })

          // Simply run git worktree remove from the main repository
          await execAsync(`git worktree remove --force "${this.workspacePath}"`, {
            cwd: this.repositoryConfig.path,
            timeout: 10000,
          })

          this.logger.info('Git worktree removed successfully')

          // If we created a branch for this worktree, try to delete it
          const branchToDelete = this.branch || (await this.detectBranchFromWorktree())
          if (!this.branchName && branchToDelete) {
            try {
              await execAsync(`git branch -D "${branchToDelete}"`, {
                cwd: this.repositoryConfig.path,
                timeout: 5000,
              })
              this.logger.debug('Branch deleted successfully', { branch: branchToDelete })
            } catch (error) {
              this.logger.debug('Branch deletion failed (best effort)', {
                branch: branchToDelete,
                error: error instanceof Error ? error.message : String(error),
              })
            }
          }

          // Run prune to clean up any stale entries
          const { pruneWorktrees } = await import('../utils/git-utils.js')
          await pruneWorktrees(this.repositoryConfig.path)
        } else {
          this.logger.debug('Workspace is not a git worktree, skipping removal', {
            workspacePath: this.workspacePath,
          })
        }
      } catch (error) {
        this.logger.error('Error during worktree cleanup', error as Error)
        // Don't throw - we want to continue with cleanup even if this fails
      }
    }

    const teardownTime = Date.now() - startTime
    this.logger.info('Workspace teardown completed', { teardownTimeMs: teardownTime })
  }

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        400,
      )
    }

    return await searchFilesInDirectoryNew(this.workspacePath, request)
  }

  async getGitDiff(): Promise<GitDiffResponse> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        400,
      )
    }

    return getGitDiffDetailed(this.workspacePath, this.originalBranch)
  }

  async runCommand(command: string, timeout?: number): Promise<CommandResult> {
    return this.environment.runCommand(command, timeout)
  }

  isRunning(): boolean {
    return this.environment.isRunning()
  }

  getStatus(): WorkspaceStatus {
    // Check if agent is running
    if (this.runningAgent) {
      return WorkspaceStatus.RUNNING
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

  async getGitStatus(): Promise<GitStatus> {
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
        hasConflicts: false,
        status: 'synced',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    try {
      // Import the functions from git-utils
      const { getOriginSyncStatus, checkForConflicts, getGitDiffDetailed } = await import(
        '../utils/git-utils.js'
      )
      const [syncStatus, hasConflicts, gitDiff] = await Promise.all([
        getOriginSyncStatus(this.workspacePath, this.originalBranch),
        checkForConflicts(this.workspacePath, this.originalBranch),
        getGitDiffDetailed(this.workspacePath, this.originalBranch),
      ])

      // Count file changes by type
      let additions = 0
      let deletions = 0
      let modifications = 0

      for (const file of gitDiff.files) {
        if (file.status === 'added') {
          additions++
        } else if (file.status === 'deleted') {
          deletions++
        } else if (file.status === 'modified') {
          modifications++
        }
      }

      return {
        currentBranch: syncStatus.currentBranch,
        baseBranch: syncStatus.baseBranch,
        hasUncommittedChanges: syncStatus.hasUncommittedChanges,
        hasConflicts,
        status: syncStatus.status,
        commitsAhead: syncStatus.commitsAhead,
        commitsBehind: syncStatus.commitsBehind,
        originExists: syncStatus.originExists,
        additions,
        deletions,
        modifications,
      }
    } catch (_error) {
      // Fall back to defaults on error
      return {
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  async getGitChanges(): Promise<GitChanges> {
    // If workspace doesn't exist or is not set up, return defaults
    if (
      !this.workspacePath ||
      !(await this.pathExists(this.workspacePath)) ||
      this.getStatus() === WorkspaceStatus.NOT_SETUP
    ) {
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    try {
      // Import and use the working directory changes function
      const { getWorkingDirectoryChanges } = await import('../utils/git-utils.js')
      return getWorkingDirectoryChanges(this.workspacePath)
    } catch (_error) {
      // Fall back to defaults on error
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  async stopAgent(): Promise<boolean> {
    return this.environment.stopAgent()
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
            files.push(`${relativePath}/`)
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
      if ((error as { code?: string }).code === 'ENOENT') {
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
      if ((error as { code?: string }).code === 'ENOENT') {
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

  // Server methods - now delegated to environment
  async startServer(): Promise<ServerStatus> {
    return this.environment.startServer!()
  }

  async stopServer(): Promise<boolean> {
    return this.environment.stopServer!()
  }

  async getServerStatus(): Promise<ServerStatus> {
    return this.environment.getServerStatus!()
  }

  async getServerLogs(lines = 100): Promise<string> {
    return this.environment.getServerLogs!(lines)
  }

  toWorkspaceInfo(): Workspace {
    return {
      id: this.taskId,
      type: 'local' as const,
      repositoryId: this.repositoryConfig.id,
      status: this.status,
      createdAt: this.createdAt,
      path: this.workspacePath,
      error: this.error,
    }
  }

  getWorkspacePath(): string {
    return this.workspacePath
  }

  setRunningAgent(agent: BaseAgent): void {
    this.environment.setRunningAgent(agent)
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
      if (this.repositoryConfig.useGitWorktrees) {
        return await isGitRepository(this.workspacePath)
      }

      return true
    } catch {
      return false
    }
  }
}
