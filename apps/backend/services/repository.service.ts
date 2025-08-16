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
import { getDataDir } from '../utils/data-dir.js'
import { isRegularGitRepository } from '../utils/git-utils.js'
import { searchFilesInDirectoryNew } from '../utils/index.js'
import { logger } from '../utils/logger.js'
import type { SettingsService } from './settings.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export class RepositoryService {
  private workspaceService?: WorkspaceService
  private claudeService?: ClaudeService
  private settingsService?: SettingsService

  constructor(private storage: StorageBackend) {}

  // Setter for WorkspaceService to avoid circular dependency
  setWorkspaceService(workspaceService: WorkspaceService): void {
    this.workspaceService = workspaceService
  }

  // Setter for ClaudeService
  setClaudeService(claudeService: ClaudeService): void {
    this.claudeService = claudeService
  }

  // Setter for SettingsService
  setSettingsService(settingsService: SettingsService): void {
    this.settingsService = settingsService
  }

  async createRepository(request: CreateRepositoryRequest): Promise<RepositoryConfig> {
    let finalPath = request.path
    let finalName = request.name

    // Handle GitHub repository
    if (request.sourceType === 'github' && request.remoteUrl) {
      const cloneResult = await this.cloneGitHubRepository(request.remoteUrl, request.name)
      finalPath = cloneResult.path
      finalName = cloneResult.name
    } else if (!request.path && request.templatePath) {
      // Handle template-based repository creation
      const repoDir =
        this.settingsService?.getRepositoriesDir() ||
        process.env.REPO_DIR ||
        path.join(getDataDir(), 'repos')

      // Create target directory path
      finalPath = path.join(repoDir, finalName)

      // Ensure repos directory exists
      await fs.mkdir(repoDir, { recursive: true })

      // Copy template to new repository directory
      await this.copyDirectory(request.templatePath, finalPath)
    } else if (!request.path) {
      throw new ValidationError('Path is required for local repositories', { path: request.path })
    }

    // Validate repository configuration
    await this.validateRepository({ ...request, path: finalPath })

    // Auto-detect framework for local repositories if not provided
    const previewConfig = request.previewConfig

    const repository: RepositoryConfig = {
      id: randomUUID(),
      name: finalName,
      description: request.description,
      path: finalPath!,
      remoteUrl: request.remoteUrl,
      sourceType: request.sourceType || 'local',
      useGitWorktrees: request.useGitWorktrees ?? (request.sourceType === 'github' ? false : false),
      originalBranch: request.originalBranch,
      templateId: request.templateId,
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
    const repository = await this.storage.getRepository(repoId)
    // Return null for soft-deleted repositories
    if (repository?.deleted) {
      return null
    }
    return repository
  }

  async listRepositories(): Promise<RepositoryConfig[]> {
    const keys = await this.storage.list('repositories/')
    const repositories: RepositoryConfig[] = []

    for (const key of keys) {
      const repoId = key.replace('repositories/', '')
      const repo = await this.storage.getRepository(repoId)
      // Filter out soft-deleted repositories
      if (repo && !repo.deleted) repositories.push(repo)
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
    // Get repository directly from storage to check if it exists (including deleted ones)
    const repository = await this.storage.getRepository(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // Check if already deleted
    if (repository.deleted) {
      return // Already deleted, nothing to do
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

    // Perform soft deletion by setting the deleted flag
    const updatedRepository = {
      ...repository,
      deleted: true,
      updatedAt: new Date(),
    }

    await this.saveRepository(updatedRepository)
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
        .map((line) => line.replace(/^[*+]\s*/, '')) // Remove current branch marker (* or +)
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
    // Skip validation for GitHub repos that haven't been cloned yet
    if (repository.sourceType === 'github' && !repository.path) {
      return
    }

    // Validate path exists for local repos or already cloned GitHub repos
    if (repository.path) {
      await this.validateLocalRepository({ path: repository.path })
    } else {
      throw new ValidationError('Path is required for local repositories', { repository })
    }
  }

  /**
   * Generate a unique clone path by adding numeric suffix if directory exists
   */
  private async generateUniqueClonePath(
    baseDir: string,
    baseName: string,
  ): Promise<{ path: string; name: string }> {
    const basePath = path.join(baseDir, baseName)

    try {
      await fs.access(basePath)
      // Directory exists, try with numeric suffixes
      let counter = 2
      while (true) {
        const uniqueName = `${baseName}-${counter}`
        const uniquePath = path.join(baseDir, uniqueName)

        try {
          await fs.access(uniquePath)
          // This path also exists, try next number
          counter++
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
            // Path doesn't exist, we can use it
            return { path: uniquePath, name: uniqueName }
          }
          throw error
        }
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        // Base path doesn't exist, we can use it
        return { path: basePath, name: baseName }
      }
      throw error
    }
  }

  /**
   * Clone a GitHub repository using the PAT from settings
   */
  private async cloneGitHubRepository(
    remoteUrl: string,
    suggestedName?: string,
  ): Promise<{ path: string; name: string }> {
    if (!this.settingsService) {
      throw new Error('SettingsService not available')
    }

    // Parse GitHub URL to extract owner and repo name
    // Support both HTTPS and SSH URLs
    const httpsPattern = /(?:https?:\/\/)?(?:www\.)?github\.com\/([^/]+)\/([^/.]+)(?:\.git)?$/
    const sshPattern = /^git@github\.com:([^/]+)\/([^/.]+?)(?:\.git)?$/

    let owner: string
    let repoName: string
    let isSSH = false

    // Try HTTPS pattern first
    let match = remoteUrl.match(httpsPattern)
    if (match) {
      ;[, owner, repoName] = match
    } else {
      // Try SSH pattern
      match = remoteUrl.match(sshPattern)
      if (match) {
        ;[, owner, repoName] = match
        isSSH = true
      } else {
        throw new ValidationError('Invalid GitHub URL format', { url: remoteUrl })
      }
    }

    let finalName = suggestedName || repoName

    // Get GitHub PAT from settings
    const githubSettings = this.settingsService?.getGitHubSettings()
    const pat = githubSettings?.personalAccessToken

    // Determine clone directory
    const repoDir =
      this.settingsService?.getRepositoriesDir() ||
      process.env.REPO_DIR ||
      path.join(getDataDir(), 'repos')

    // Ensure repos directory exists
    await fs.mkdir(repoDir, { recursive: true })

    // Generate unique clone path (handles conflicts automatically)
    const uniqueCloneInfo = await this.generateUniqueClonePath(repoDir, finalName)
    const clonePath = uniqueCloneInfo.path
    finalName = uniqueCloneInfo.name

    // Build clone URL with authentication if needed
    let cloneUrl = remoteUrl

    if (isSSH) {
      // SSH URLs use SSH keys for authentication, no need to modify the URL
      // The system's SSH configuration will handle authentication
      logger.info('Using SSH URL for cloning - ensure SSH keys are configured', {
        owner,
        repo: repoName,
        path: clonePath,
      })
    } else if (pat) {
      // For HTTPS URLs, add PAT authentication if available
      const urlWithAuth = remoteUrl.replace(/^https:\/\/github\.com/, `https://${pat}@github.com`)
      cloneUrl = urlWithAuth
    }

    logger.info('Cloning GitHub repository', {
      owner,
      repo: repoName,
      path: clonePath,
      isSSH,
      hasAuth: !isSSH && !!pat,
    })

    // Clone the repository with SSH workaround for systemd services
    let cloneCommand = `git clone "${cloneUrl}" "${finalName}"`

    // Apply SSH workaround if SSH URL and we detect permission issues
    // This is needed for systemd services or containerized environments
    if (isSSH) {
      // First, try to detect if we'll have permission issues
      const testResult = await this.executeCommand('cat ~/.ssh/known_hosts 2>&1', repoDir)

      if (!testResult.success || testResult.error?.includes('Permission denied')) {
        // Permission issue detected - likely running under systemd
        // Use SSH key from data directory with host checking bypass
        const dataDir = getDataDir()
        const serviceSSHDir = `${path.dirname(dataDir)}/.ssh`

        // Check for ed25519 key first, then RSA
        let sshKeyPath = `${serviceSSHDir}/id_ed25519`
        const keyCheckResult = await this.executeCommand(
          `test -f ${sshKeyPath} && echo "exists" || echo "not found"`,
          repoDir,
        )

        if (keyCheckResult.output?.includes('not found')) {
          // Try RSA key as fallback
          sshKeyPath = `${serviceSSHDir}/id_rsa`
        }

        cloneCommand = `export GIT_SSH_COMMAND="ssh -i ${sshKeyPath} -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no" && ${cloneCommand}`
        logger.info('Using service SSH key for systemd environment', {
          sshKeyPath,
          keyExists: keyCheckResult.output,
        })
      } else {
        logger.info('SSH permissions OK, proceeding with normal clone')
      }
    }

    const cloneResult = await this.executeCommand(cloneCommand, repoDir)

    if (!cloneResult.success) {
      // Log the full error for debugging
      logger.error('Git clone failed', new Error(cloneResult.error || 'Unknown error'), {
        command: isSSH ? 'git clone [SSH URL]' : 'git clone [HTTPS URL]',
        exitCode: cloneResult.exit_code,
        output: cloneResult.output,
        isSSH,
      })

      // Remove PAT from error message for security
      const sanitizedError = cloneResult.error?.replace(pat || '', '***')

      let errorMessage = `Failed to clone repository: ${sanitizedError}`
      if (isSSH) {
        errorMessage += '\n\nFor SSH URLs, ensure you have SSH keys configured for GitHub access.'
      } else if (!pat) {
        errorMessage +=
          '\n\nFor HTTPS URLs, consider adding a GitHub Personal Access Token in settings.'
      }

      throw new ValidationError(errorMessage, {
        url: remoteUrl,
        error: sanitizedError,
        isSSH,
      })
    }

    // For HTTPS URLs with PAT, remove the PAT from the remote URL for security
    // SSH URLs don't need this step
    if (!isSSH && pat) {
      await this.executeCommand(`git remote set-url origin "${remoteUrl}"`, clonePath)
    }

    logger.info('Successfully cloned GitHub repository', {
      name: finalName,
      path: clonePath,
    })

    return {
      path: clonePath,
      name: finalName,
    }
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
   * Copy directory contents from source to destination, excluding .git and node_modules
   */
  private async copyDirectory(src: string, dest: string): Promise<void> {
    await fs.mkdir(dest, { recursive: true })
    const entries = await fs.readdir(src, { withFileTypes: true })

    for (const entry of entries) {
      const srcPath = path.join(src, entry.name)
      const destPath = path.join(dest, entry.name)

      if (entry.isDirectory()) {
        // Skip directories that shouldn't be copied
        if (entry.name === '.git' || entry.name === 'node_modules') continue
        await this.copyDirectory(srcPath, destPath)
      } else if (entry.isFile()) {
        try {
          await fs.copyFile(srcPath, destPath)
        } catch (error) {
          // Log warning for files that can't be copied (symlinks, special files, etc.)
          logger.warn('Failed to copy file, skipping', {
            srcPath,
            destPath,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      }
      // Skip symlinks and other special file types
    }

    // Initialize git in the new repository
    await this.executeCommand('git init', dest)
    await this.executeCommand('git add .', dest)
    await this.executeCommand('git commit -m "Initial commit from template"', dest)
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
