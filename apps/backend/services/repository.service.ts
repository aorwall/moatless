import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  FileSearchResponse,
  SetupScriptResponse,
  WorkspaceStatusResponse,
} from '@moatless/types'
import {
  type CreateRepositoryRequest,
  FRAMEWORK_DEFAULT_COMMANDS,
  PACKAGE_MANAGER_FILES,
  type PreviewConfig,
  type RepositoryConfig,
  RepositoryNotFoundError,
  ValidationError,
} from '@moatless/types'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import { isRegularGitRepository } from '../utils/git-utils.js'
import { searchFilesInDirectoryNew } from '../utils/index.js'
import { logger } from '../utils/logger.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export class RepositoryService {
  private workspaceService?: WorkspaceService
  private claudeService?: ClaudeService

  constructor(private storage: StorageBackend) {}

  // Setter for WorkspaceService to avoid circular dependency
  setWorkspaceService(workspaceService: WorkspaceService): void {
    this.workspaceService = workspaceService
  }

  // Setter for ClaudeService
  setClaudeService(claudeService: ClaudeService): void {
    this.claudeService = claudeService
  }

  async createRepository(request: CreateRepositoryRequest): Promise<RepositoryConfig> {
    // Validate repository configuration
    await this.validateRepository(request)

    // Auto-detect framework for local repositories if not provided
    const previewConfig = request.previewConfig

    const repository: RepositoryConfig = {
      id: randomUUID(),
      name: request.name,
      description: request.description,
      path: request.path,
      useGitWorktrees: request.useGitWorktrees,
      originalBranch: request.originalBranch,
      codeExecution: request.codeExecution,
      previewConfig,
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)
    return repository
  }

  async saveRepository(repo: RepositoryConfig): Promise<void> {
    repo.updatedAt = new Date()
    await this.storage.saveRepository(repo)
  }

  async getRepositoryById(repoId: string): Promise<RepositoryConfig | null> {
    return await this.storage.getRepository(repoId)
  }

  async listRepositories(): Promise<RepositoryConfig[]> {
    const keys = await this.storage.list('repositories/')
    const repositories: RepositoryConfig[] = []

    for (const key of keys) {
      const repoId = key.replace('repositories/', '')
      const repo = await this.storage.getRepository(repoId)
      if (repo) repositories.push(repo)
    }

    // Sort by creation date (newest first)
    return repositories.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
  }

  async updateRepository(
    repoId: string,
    updates: Partial<RepositoryConfig>,
  ): Promise<RepositoryConfig> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // Validate repository if path is being updated
    if (updates.path) {
      await this.validateRepository(updates as CreateRepositoryRequest)
    }

    const updatedRepository = {
      ...repository,
      ...updates,
      id: repoId, // Ensure ID doesn't change
      updatedAt: new Date(),
    }

    await this.saveRepository(updatedRepository)
    return updatedRepository
  }

  async deleteRepository(repoId: string): Promise<void> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // Check if repository is in use by any open or running tasks
    const tasks = await this.storage.list('tasks/')
    for (const taskKey of tasks) {
      const taskId = taskKey.replace('tasks/', '')
      const task = await this.storage.getTask(taskId)
      if (task && task.repositoryId === repoId && task.status === 'open') {
        throw new ValidationError(
          `Cannot delete repository ${repoId} - it is in use by task ${taskId}`,
          { repositoryId: repoId, taskId },
        )
      }
    }

    await this.storage.delete(`repositories/${repoId}`)
  }

  async runSetupScript(repository: RepositoryConfig): Promise<SetupScriptResponse> {
    if (
      !repository.codeExecution.setupCommands ||
      repository.codeExecution.setupCommands.length === 0
    ) {
      return {
        success: true,
        commands: [],
      }
    }

    const commands = repository.codeExecution.setupCommands

    const results = []
    let allSucceeded = true

    for (const command of commands) {
      try {
        // Execute command in repository path
        const result = await this.executeCommand(command, repository.path)
        results.push(result)
        if (!result.success) {
          allSucceeded = false
        }
      } catch (error) {
        const errorResult = {
          command,
          success: false,
          exit_code: 1,
          error: error instanceof Error ? error.message : String(error),
        }
        results.push(errorResult)
        allSucceeded = false
      }
    }

    return {
      success: allSucceeded,
      commands: results,
    }
  }

  async getBranches(repoPath: string): Promise<string[]> {
    try {
      const result = await this.executeCommand('git branch -a', repoPath)
      if (!result.success || !result.output) {
        return []
      }

      const branches = result.output
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.length > 0)
        .map((line) => line.replace(/^\*\s*/, '')) // Remove current branch marker
        .map((line) => line.replace(/^remotes\/origin\//, '')) // Remove remote prefix
        .filter((line) => !line.includes('HEAD ->')) // Remove HEAD pointer
        .filter((line) => !line.startsWith('workspace-')) // Filter out workspace branches

      // Deduplicate branches
      const uniqueBranches = [...new Set(branches)]

      // Sort branches: main/master first, then alphabetically
      return uniqueBranches.sort((a, b) => {
        const isMainA = a === 'main' || a === 'master'
        const isMainB = b === 'main' || b === 'master'

        if (isMainA && !isMainB) return -1
        if (!isMainA && isMainB) return 1
        if (isMainA && isMainB) {
          // Both are main branches, prefer 'main' over 'master'
          return a === 'main' ? -1 : 1
        }

        return a.localeCompare(b)
      })
    } catch {
      return []
    }
  }

  async searchFiles(
    repoId: string,
    request: { query: string; limit?: number },
  ): Promise<FileSearchResponse> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    return await searchFilesInDirectoryNew(repository.path, request)
  }

  async validateRepository(repository: CreateRepositoryRequest | RepositoryConfig): Promise<void> {
    // Validate path exists
    await this.validateLocalRepository(repository)
  }

  private async validateLocalRepository(repo: { path: string }): Promise<void> {
    try {
      const stats = await fs.stat(repo.path)
      if (!stats.isDirectory()) {
        throw new ValidationError(`Path ${repo.path} is not a directory`, { path: repo.path })
      }

      // Check if it's a git repository
      const gitPath = path.join(repo.path, '.git')
      try {
        await fs.access(gitPath)
      } catch {
        throw new ValidationError(`Path ${repo.path} is not a git repository`, { path: repo.path })
      }
    } catch (error) {
      if (error instanceof ValidationError) {
        throw error
      }
      throw new ValidationError(
        `Cannot access repository at ${repo.path}: ${error instanceof Error ? error.message : String(error)}`,
        { path: repo.path, error: error instanceof Error ? error.message : String(error) },
      )
    }
  }

  private async executeCommand(
    command: string,
    cwd: string,
  ): Promise<{
    command: string
    success: boolean
    exit_code: number
    output?: string
    error?: string
  }> {
    const { spawn } = await import('node:child_process')

    return new Promise((resolve) => {
      const [cmd, ...args] = command.split(' ')
      const child = spawn(cmd, args, {
        cwd,
        stdio: ['pipe', 'pipe', 'pipe'],
        shell: true,
      })

      let stdout = ''
      let stderr = ''

      child.stdout?.on('data', (data) => {
        stdout += data.toString()
      })

      child.stderr?.on('data', (data) => {
        stderr += data.toString()
      })

      child.on('close', (code) => {
        resolve({
          command,
          success: code === 0,
          exit_code: code || 0,
          output: stdout.trim() || undefined,
          error: stderr.trim() || undefined,
        })
      })

      child.on('error', (error) => {
        resolve({
          command,
          success: false,
          exit_code: 1,
          error: error.message,
        })
      })
    })
  }

  /**
   * Detect the package manager from lock files
   */
  private async detectPackageManager(repoPath: string): Promise<string> {
    for (const { file, manager } of PACKAGE_MANAGER_FILES) {
      try {
        await fs.access(path.join(repoPath, file))
        return manager
      } catch {
        // File doesn't exist, continue
      }
    }
    return 'npm' // Default fallback
  }

  /**
   * Detect the framework type and version from a local repository path
   */
  async detectFramework(repoPath: string): Promise<Partial<PreviewConfig>> {
    const packageJsonPath = path.join(repoPath, 'package.json')

    try {
      // Detect package manager and read package.json
      const packageManager = await this.detectPackageManager(repoPath)
      const packageJson = JSON.parse(await fs.readFile(packageJsonPath, 'utf-8'))
      const deps = { ...packageJson.dependencies, ...packageJson.devDependencies }
      const scripts = packageJson.scripts || {}

      // Helper to get default command with detected package manager
      const getDefaultCommand = (framework: keyof typeof FRAMEWORK_DEFAULT_COMMANDS): string => {
        return FRAMEWORK_DEFAULT_COMMANDS[framework].replace('npm', packageManager)
      }

      // Detect framework based on dependencies
      if (deps.vite) {
        const serverCommand =
          this.detectServerCommand(scripts, ['dev', 'start', 'serve']) || getDefaultCommand('vite')
        return {
          framework: 'vite',
          frameworkVersion: deps.vite,
          serverStartCommand: serverCommand,
          injectionStrategy: {
            method: 'html-injection',
            config: {
              indexPath: 'index.html',
              insertLocation: 'head',
            },
          },
        }
      }

      if (deps.next) {
        const serverCommand =
          this.detectServerCommand(scripts, ['dev', 'start']) || getDefaultCommand('nextjs')
        return {
          framework: 'nextjs',
          frameworkVersion: deps.next,
          serverStartCommand: serverCommand,
          injectionStrategy: {
            method: 'middleware',
            config: {
              middleware: 'api',
            },
          },
        }
      }

      if (deps['react-scripts']) {
        const serverCommand =
          this.detectServerCommand(scripts, ['start', 'dev']) || getDefaultCommand('cra')
        return {
          framework: 'cra',
          frameworkVersion: deps['react-scripts'],
          serverStartCommand: serverCommand,
          injectionStrategy: {
            method: 'html-injection',
            config: {
              indexPath: 'public/index.html',
              insertLocation: 'head',
            },
          },
        }
      }

      if (deps.webpack) {
        const serverCommand =
          this.detectServerCommand(scripts, ['dev', 'start', 'serve']) ||
          getDefaultCommand('webpack')
        return {
          framework: 'webpack',
          frameworkVersion: deps.webpack,
          serverStartCommand: serverCommand,
          injectionStrategy: {
            method: 'html-injection',
            config: {},
          },
        }
      }

      // Unknown framework, try to detect start command
      const serverCommand =
        this.detectServerCommand(scripts, ['dev', 'start', 'serve']) || getDefaultCommand('unknown')
      return {
        framework: 'unknown',
        serverStartCommand: serverCommand,
        injectionStrategy: {
          method: 'html-injection',
          config: {},
        },
      }
    } catch (error) {
      logger.debug('Failed to detect framework', { repoPath, error })
      // Try to detect package manager even if package.json parsing failed
      const packageManager = await this.detectPackageManager(repoPath).catch(() => 'npm')
      return {
        framework: 'unknown',
        serverStartCommand: FRAMEWORK_DEFAULT_COMMANDS.unknown.replace('npm', packageManager),
        injectionStrategy: {
          method: 'html-injection',
          config: {},
        },
      }
    }
  }

  /**
   * Detect the server start command from package.json scripts
   */
  private detectServerCommand(
    scripts: Record<string, string>,
    preferredCommands: string[],
  ): string | undefined {
    // Check preferred commands in order
    for (const cmd of preferredCommands) {
      if (scripts[cmd]) {
        return `npm run ${cmd}`
      }
    }

    // Look for any script containing 'dev' or 'start'
    for (const [scriptName, _scriptValue] of Object.entries(scripts)) {
      if (scriptName.includes('dev') || scriptName.includes('start')) {
        return `npm run ${scriptName}`
      }
    }

    return undefined
  }

  /**
   * Get suggested repositories from Claude projects
   */
  async getSuggestedRepositories(onlyGitRepos = true): Promise<{ name: string; path: string }[]> {
    if (!this.claudeService) {
      throw new Error('ClaudeService not available')
    }

    try {
      // Get all project directories from Claude config
      const projectPaths = await this.claudeService.getClaudeProjectDirectories()
      const suggestions: { name: string; path: string }[] = []

      for (const projectPath of projectPaths) {
        try {
          // Check if the path exists
          const stats = await fs.stat(projectPath)
          if (!stats.isDirectory()) {
            continue
          }

          // If onlyGitRepos is true, check if it's a git repository (not a worktree)
          if (onlyGitRepos) {
            const isGitRepo = await isRegularGitRepository(projectPath)
            if (!isGitRepo) {
              continue
            }
          }

          // Extract the directory name as the repository name
          const name = path.basename(projectPath)
          suggestions.push({ name, path: projectPath })
        } catch (error) {
          // Skip paths that don't exist or can't be accessed
          logger.debug('Failed to check project path', {
            path: projectPath,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      }

      // Sort by name
      return suggestions.sort((a, b) => a.name.localeCompare(b.name))
    } catch (error) {
      logger.error('Failed to get suggested repositories', error as Error)
      return []
    }
  }

  /**
   * Verify repository setup by creating a temporary workspace and running setup commands
   */
  async verifyRepositorySetup(repoId: string): Promise<WorkspaceStatusResponse> {
    const VERIFICATION_TIMEOUT_MS = 180000 // 3 minutes
    const POLL_INTERVAL_MS = 2000 // 2 seconds

    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    if (!this.workspaceService) {
      throw new Error('WorkspaceService not available for verification')
    }

    const fakeTaskId = `verify-${Date.now()}`
    let workspace: any = null

    try {
      // Log setup commands if missing
      if (!repository.codeExecution.setupCommands) {
        logger.info('No setup commands configured for repository', {
          taskId: fakeTaskId,
          repositoryId: repoId,
        })
      }

      // Create workspace and environment
      workspace = await this.workspaceService.createWorkspace(fakeTaskId, repoId)
      await workspace.setup()

      // Poll for status until ready, error, or timeout
      const startTime = Date.now()
      let lastStatus = ''

      logger.info('Starting repository verification polling', {
        taskId: fakeTaskId,
        repositoryId: repoId,
        timeoutMs: VERIFICATION_TIMEOUT_MS,
      })

      while (Date.now() - startTime < VERIFICATION_TIMEOUT_MS) {
        const workspaceStatus = await workspace.getWorkspaceStatus()
        const currentStatus = workspaceStatus.environment.status

        // Log status changes
        if (currentStatus !== lastStatus) {
          logger.info('Repository verification status change', {
            taskId: fakeTaskId,
            repositoryId: repoId,
            previousStatus: lastStatus || 'unknown',
            currentStatus,
            elapsedMs: Date.now() - startTime,
          })
          lastStatus = currentStatus
        }

        // Check if we're done
        if (currentStatus === 'ready') {
          logger.info('Repository verification completed successfully', {
            taskId: fakeTaskId,
            repositoryId: repoId,
            totalTimeMs: Date.now() - startTime,
          })
          return workspaceStatus
        }

        if (currentStatus === 'error') {
          const error = workspaceStatus.environment.error || 'Unknown error'
          logger.error('Repository verification failed', new Error(error), {
            taskId: fakeTaskId,
            repositoryId: repoId,
            totalTimeMs: Date.now() - startTime,
          })
          throw new Error(`Repository setup failed: ${error}`)
        }

        // Wait before next poll
        await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS))
      }

      // Timeout reached
      logger.error('Repository verification timed out', new Error('Verification timeout'), {
        taskId: fakeTaskId,
        repositoryId: repoId,
        timeoutMs: VERIFICATION_TIMEOUT_MS,
        lastStatus,
      })
      throw new Error(`Repository verification timed out after ${VERIFICATION_TIMEOUT_MS}ms`)
    } finally {
      // Clean up the verification workspace
      if (workspace) {
        await this.workspaceService.cleanupWorkspace(fakeTaskId)
      }
    }
  }
}

export function createRepositoryService(storage: StorageBackend): RepositoryService {
  return new RepositoryService(storage)
}
