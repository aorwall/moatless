import type { ChildProcess } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CodeExecution, PreviewConfig, RepositoryConfigFile } from '@moatless/types'
import {
  type CreateRepositoryRequest,
  type RepositoryConfig,
  RepositoryConfigFileSchema,
  RepositoryConfigSchema,
  RepositoryNotFoundError,
  type RepositoryTemplate,
  type UpdateRepositoryRequest,
  ValidationError,
} from '@moatless/types'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import { TEMPLATES } from '../config/templates.js'
import type { StorageBackend } from '../storage/index.js'
import { getDataDir } from '../utils/data-dir.js'
import { pathExists, readFileSafe, writeFileSafe } from '../utils/filesystem.js'
import { cloneRepository, isRegularGitRepository } from '../utils/git-utils.js'
import { logger } from '../utils/logger.js'
import { getGitHubService } from './github.service.js'
import type { SettingsService } from './settings.service.js'

export class RepositoryService {
  private claudeService?: ClaudeService
  private settingsService?: SettingsService

  constructor(private storage: StorageBackend) {}

  // Setter for ClaudeService
  setClaudeService(claudeService: ClaudeService): void {
    this.claudeService = claudeService
  }

  // Setter for SettingsService
  setSettingsService(settingsService: SettingsService): void {
    this.settingsService = settingsService
  }

  /**
   * Get a template by ID
   */
  private getTemplate(templateId: string): RepositoryTemplate {
    const template = TEMPLATES.find((t) => t.id === templateId)
    if (!template) {
      throw new ValidationError(`Template with id ${templateId} not found`, {
        templateId,
      })
    }
    return template
  }

  private getRepositoriesDirectory(): string {
    return (
      this.settingsService?.getRepositoriesDir() ||
      process.env['REPO_DIR'] ||
      path.join(getDataDir(), 'repos')
    )
  }

  private async resolveRepositoryPath(
    request: CreateRepositoryRequest,
    sessionToken: string | undefined,
  ): Promise<{ path: string; name: string; sourceType?: 'github' }> {
    if (request.sourceType === 'github' && request.remoteUrl) {
      const cloneResult = await this.cloneGitRepository(
        request.remoteUrl,
        request.name,
        sessionToken,
      )
      return { ...cloneResult, sourceType: 'github' }
    }

    if (!request.path) {
      throw new ValidationError('Path is required for local repositories', {
        path: request.path,
      })
    }

    return { path: request.path, name: request.name }
  }

  /**
   * Create a repository from a template by cloning it fresh
   */
  async createRepositoryFromTemplate(
    repoName: string,
    templateId: string,
  ): Promise<RepositoryConfig> {
    const template = await this.getTemplate(templateId)
    const repoDir = this.getRepositoriesDirectory()
    const targetPath = path.join(repoDir, repoName)

    await fs.mkdir(repoDir, { recursive: true })

    // Clone template directly to target path
    logger.info('Cloning template repository', {
      templateId: template.id,
      templateName: template.name,
      gitUrl: template.gitUrl,
      targetPath,
    })

    await cloneRepository(template.gitUrl, targetPath)

    // Remove remote origin to disconnect from template repository
    await this.executeCommand('git remote remove origin', targetPath).catch(() => {
      // Ignore error if no remote origin exists
    })

    // Add empty commit to mark this as forked from template
    await this.executeCommand(
      `git commit --allow-empty -m "Forked from template: ${template.gitUrl}"`,
      targetPath,
    )

    logger.info('Successfully created repository from template', {
      templateId: template.id,
      templateName: template.name,
      targetPath,
    })

    // Merge template config with defaults
    const codeExecution: CodeExecution = {
      type: 'local',
      setupCommands: template.setupCommand ? [template.setupCommand] : undefined,
    }

    const previewConfig: PreviewConfig = {
      ...template.previewConfig,
      takeScreenshots: true,
      enableInspector: true,
    }

    const repository: RepositoryConfig = {
      id: randomUUID(),
      name: repoName,
      description: `Created from template: ${template.name}`,
      path: targetPath,
      sourceType: 'template',
      useGitWorktrees: false,
      cloneOnNewTask: false,
      templateId,
      vibe: true,
      codeExecution,
      previewConfig,
      systemPrompt: template.systemPrompt,
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)
    return repository
  }

  async createRepository(
    request: CreateRepositoryRequest,
    sessionToken?: string,
  ): Promise<RepositoryConfig> {
    const resolved = await this.resolveRepositoryPath(request, sessionToken)

    await this.validateRepository({ ...request, path: resolved.path })

    if (request.cloneOnNewTask && !request.remoteUrl) {
      throw new ValidationError(
        'cloneOnNewTask requires a remoteUrl to be configured. Cannot enable cloning without remote URL.',
        {
          cloneOnNewTask: request.cloneOnNewTask,
          remoteUrl: request.remoteUrl,
        },
      )
    }

    // Auto-detect if repository has a config file, unless explicitly set
    let syncWithRepoConfig = request.syncWithRepoConfig
    if (syncWithRepoConfig === undefined) {
      const hasConfigFile = await this.detectRepoConfigFile(resolved.path)
      syncWithRepoConfig = hasConfigFile
      if (hasConfigFile) {
        logger.info('Auto-detected .moatless/config.json, enabling config sync', {
          path: resolved.path,
        })
      }
    }

    let repository: RepositoryConfig = {
      id: randomUUID(),
      name: resolved.name,
      description: request.description,
      path: resolved.path,
      remoteUrl: request.remoteUrl,
      sourceType: resolved.sourceType || request.sourceType || 'local',
      useGitWorktrees: request.useGitWorktrees ?? false,
      cloneOnNewTask: request.cloneOnNewTask,
      originalBranch: request.originalBranch,
      vibe: request.vibe,
      syncWithRepoConfig,
      codeExecution: request.codeExecution,
      previewConfig: request.previewConfig,
      systemPrompt: request.systemPrompt,
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    // If sync is enabled, merge config from file
    if (syncWithRepoConfig) {
      repository = await this.mergeWithRepoConfig(repository)
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
    // Parse through schema to ensure legacy repositories get default values
    if (repository) {
      const parsed = RepositoryConfigSchema.parse(repository)
      // If sync is enabled, merge config from file
      return await this.mergeWithRepoConfig(parsed)
    }
    return repository
  }

  async listRepositories(includeVibeRepos = false): Promise<RepositoryConfig[]> {
    const keys = await this.storage.list('repositories/')
    const repositories: RepositoryConfig[] = []

    for (const key of keys) {
      const repoId = key.replace('repositories/', '')
      const repo = await this.storage.getRepository(repoId)
      // Filter out soft-deleted repositories
      if (repo && !repo.deleted && (!repo.vibe || includeVibeRepos)) {
        // Filter out vibe repositories unless explicitly requested
        repositories.push(repo)
      }
    }

    // Sort by creation date (newest first)
    return repositories.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
  }

  /**
   * Handle sync config toggle when updating repository
   */
  private handleSyncConfigToggle(
    repoId: string,
    repository: RepositoryConfig,
    updates: UpdateRepositoryRequest,
  ): void {
    const currentSyncEnabled = repository.syncWithRepoConfig
    const newSyncEnabled =
      updates.syncWithRepoConfig !== undefined ? updates.syncWithRepoConfig : currentSyncEnabled

    if (currentSyncEnabled && !newSyncEnabled) {
      // Transitioning from synced to not synced - keep the current merged values
      logger.info('Disabling config sync, preserving current values', { repositoryId: repoId })
      return
    }

    if (!currentSyncEnabled && newSyncEnabled) {
      // Transitioning from not synced to synced
      // Config file will be written after the repository update with the new values
      logger.info('Enabling config sync', { repositoryId: repoId })
      return
    }
  }

  private validateCloneSettings(
    updates: UpdateRepositoryRequest,
    repository: RepositoryConfig,
  ): void {
    const finalRemoteUrl =
      updates.remoteUrl !== undefined ? updates.remoteUrl : repository.remoteUrl
    const finalCloneOnNewTask =
      updates.cloneOnNewTask !== undefined ? updates.cloneOnNewTask : repository.cloneOnNewTask

    if (finalCloneOnNewTask && !finalRemoteUrl) {
      throw new ValidationError(
        'cloneOnNewTask requires a remoteUrl to be configured. Cannot enable cloning without remote URL.',
        { cloneOnNewTask: finalCloneOnNewTask, remoteUrl: finalRemoteUrl },
      )
    }
  }

  private buildUpdatedRepository(
    repoId: string,
    repository: RepositoryConfig,
    updates: UpdateRepositoryRequest,
  ): RepositoryConfig {
    return {
      id: repoId,
      name: updates.name ?? repository.name,
      description:
        updates.description !== undefined
          ? (updates.description ?? undefined)
          : repository.description,
      path: updates.path ?? repository.path,
      remoteUrl:
        updates.remoteUrl !== undefined ? (updates.remoteUrl ?? undefined) : repository.remoteUrl,
      sourceType: updates.sourceType ?? repository.sourceType,
      useGitWorktrees: updates.useGitWorktrees ?? repository.useGitWorktrees,
      cloneOnNewTask: updates.cloneOnNewTask ?? repository.cloneOnNewTask,
      syncWithRepoConfig: updates.syncWithRepoConfig ?? repository.syncWithRepoConfig,
      codeExecution: updates.codeExecution ?? repository.codeExecution,
      previewConfig: updates.previewConfig ?? repository.previewConfig,
      systemPrompt:
        updates.systemPrompt !== undefined
          ? (updates.systemPrompt ?? undefined)
          : repository.systemPrompt,
      originalBranch:
        updates.originalBranch !== undefined
          ? (updates.originalBranch ?? undefined)
          : repository.originalBranch,
      deleted: repository.deleted,
      templateId: repository.templateId,
      vibe: repository.vibe,
      createdAt: repository.createdAt,
      updatedAt: new Date(),
    }
  }

  private async syncConfigToFile(repository: RepositoryConfig): Promise<void> {
    if (!repository.syncWithRepoConfig) {
      return
    }

    const configFile = {
      systemPrompt: repository.systemPrompt,
      codeExecution: repository.codeExecution,
      previewConfig: repository.previewConfig,
    }
    await this.writeRepoConfigFile(repository.path, configFile)
  }

  async updateRepository(
    repoId: string,
    updates: UpdateRepositoryRequest,
  ): Promise<RepositoryConfig> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // Validate repository if path is being updated
    if (updates.path) {
      await this.validateRepository(updates as CreateRepositoryRequest)
    }

    // Validate cloneOnNewTask requires remoteUrl
    this.validateCloneSettings(updates, repository)

    // Handle sync config toggle
    this.handleSyncConfigToggle(repoId, repository, updates)

    // Build updated repository
    const updatedRepository = this.buildUpdatedRepository(repoId, repository, updates)

    await this.saveRepository(updatedRepository)

    // Sync config to file if enabled
    await this.syncConfigToFile(updatedRepository)

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

  async getBranches(repoPath: string): Promise<string[]> {
    try {
      const result = await this.executeCommand('git branch -a', repoPath)
      if (!(result.success && result.output)) {
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

        if (isMainA && !isMainB) {
          return -1
        }
        if (!isMainA && isMainB) {
          return 1
        }
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

  async validateRepository(repository: CreateRepositoryRequest | RepositoryConfig): Promise<void> {
    // Skip validation for GitHub repos that haven't been cloned yet
    if (repository.sourceType === 'github' && !repository.path) {
      return
    }

    // Validate path exists for local repos or already cloned GitHub repos
    if (repository.path) {
      await this.validateLocalRepository({ path: repository.path })
    } else {
      throw new ValidationError('Path is required for local repositories', {
        repository,
      })
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
   * Clone a GitHub repository via HTTPS
   */
  private async cloneGitRepository(
    remoteUrl: string,
    suggestedName?: string,
    sessionToken?: string,
  ): Promise<{ path: string; name: string }> {
    if (!this.settingsService) {
      throw new Error('SettingsService not available')
    }

    const parsedUrl = this.parseGitUrl(remoteUrl)
    const finalName = suggestedName || parsedUrl.repoName
    const pat = this.getGitHubToken(sessionToken)

    const { clonePath } = await this.prepareCloneDirectory(finalName)
    const cloneUrl = this.buildCloneUrl(remoteUrl, pat)

    this.logCloneStart(parsedUrl, clonePath, pat)

    await this.executeClone(cloneUrl, clonePath, remoteUrl, pat)

    if (pat) {
      await this.removePATFromRemote(remoteUrl, clonePath)
    }

    logger.info('Successfully cloned GitHub repository', {
      owner: parsedUrl.owner,
      repo: parsedUrl.repoName,
      name: finalName,
      path: clonePath,
    })

    return { path: clonePath, name: finalName }
  }

  private parseGitUrl(remoteUrl: string): {
    owner: string
    repoName: string
  } {
    // Only accept GitHub HTTPS URLs
    const githubHttpsPattern = /^https:\/\/github\.com\/([^/]+)\/([^/.]+)(?:\.git)?$/

    const match = remoteUrl.match(githubHttpsPattern)
    if (match) {
      return {
        owner: match[1] || '',
        repoName: match[2] || '',
      }
    }

    throw new ValidationError(
      'Invalid GitHub URL format. Only HTTPS URLs are supported (e.g., https://github.com/owner/repo)',
      { url: remoteUrl },
    )
  }

  private getGitHubToken(sessionToken?: string): string | undefined {
    const githubSettings = this.settingsService?.getGitHubSettings(sessionToken)
    return githubSettings?.personalAccessToken
  }

  private async prepareCloneDirectory(
    finalName: string,
  ): Promise<{ clonePath: string; repoDir: string }> {
    const repoDir =
      this.settingsService?.getRepositoriesDir() ||
      process.env['REPO_DIR'] ||
      path.join(getDataDir(), 'repos')

    await fs.mkdir(repoDir, { recursive: true })

    const uniqueCloneInfo = await this.generateUniqueClonePath(repoDir, finalName)
    return { clonePath: uniqueCloneInfo.path, repoDir }
  }

  private buildCloneUrl(remoteUrl: string, pat: string | undefined): string {
    if (!pat) {
      return remoteUrl
    }

    return remoteUrl.replace(/^https:\/\/github\.com/, `https://${pat}@github.com`)
  }

  private logCloneStart(
    parsedUrl: {
      owner: string
      repoName: string
    },
    clonePath: string,
    pat: string | undefined,
  ): void {
    logger.info('Cloning GitHub repository', {
      owner: parsedUrl.owner,
      repo: parsedUrl.repoName,
      path: clonePath,
      hasAuth: Boolean(pat),
    })
  }

  private async executeClone(
    cloneUrl: string,
    clonePath: string,
    remoteUrl: string,
    pat: string | undefined,
  ): Promise<void> {
    try {
      await cloneRepository(cloneUrl, clonePath)
    } catch (error) {
      logger.error('Git clone failed', error as Error)

      const errorMessage = this.buildCloneErrorMessage(
        error instanceof Error ? error.message : String(error),
        pat,
      )

      throw new ValidationError(errorMessage, {
        url: remoteUrl,
        error: (error instanceof Error ? error.message : String(error)).replace(pat || '', '***'),
      })
    }
  }

  private buildCloneErrorMessage(error: string | undefined, pat: string | undefined): string {
    const sanitizedError = error?.replace(pat || '', '***')
    let message = `Failed to clone repository: ${sanitizedError}`

    if (!pat) {
      message +=
        '\n\nConsider adding a GitHub Personal Access Token in settings for authentication.'
    }

    return message
  }

  private async removePATFromRemote(remoteUrl: string, clonePath: string): Promise<void> {
    await this.executeCommand(`git remote set-url origin "${remoteUrl}"`, clonePath)
  }

  private async validateLocalRepository(repo: { path: string }): Promise<void> {
    try {
      const stats = await fs.stat(repo.path)
      if (!stats.isDirectory()) {
        throw new ValidationError(`Path ${repo.path} is not a directory`, {
          path: repo.path,
        })
      }

      // Check if it's a git repository
      const gitPath = path.join(repo.path, '.git')
      try {
        await fs.access(gitPath)
      } catch {
        throw new ValidationError(`Path ${repo.path} is not a git repository`, {
          path: repo.path,
        })
      }
    } catch (error) {
      if (error instanceof ValidationError) {
        throw error
      }
      throw new ValidationError(
        `Cannot access repository at ${repo.path}: ${error instanceof Error ? error.message : String(error)}`,
        {
          path: repo.path,
          error: error instanceof Error ? error.message : String(error),
        },
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
      if (!cmd) {
        return resolve({
          command,
          success: false,
          exit_code: 1,
          error: 'Invalid command',
        })
      }
      const child: ChildProcess = spawn(cmd, args, {
        cwd,
        stdio: ['pipe', 'pipe', 'pipe'],
        shell: true,
      })

      let stdout = ''
      let stderr = ''

      child.stdout?.on('data', (data: Buffer) => {
        stdout += data.toString()
      })

      child.stderr?.on('data', (data: Buffer) => {
        stderr += data.toString()
      })

      child.on('close', (code: number | null) => {
        const result: {
          command: string
          success: boolean
          exit_code: number
          output?: string
          error?: string
        } = {
          command,
          success: code === 0,
          exit_code: code || 0,
        }

        const trimmedOutput = stdout.trim()
        if (trimmedOutput) {
          result.output = trimmedOutput
        }

        const trimmedError = stderr.trim()
        if (trimmedError) {
          result.error = trimmedError
        }

        resolve(result)
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
   * Detect if a repository has a .moatless/config.json file
   * @param repoPath Local path to the repository
   * @returns True if config file exists
   */
  async detectRepoConfigFile(repoPath: string): Promise<boolean> {
    const configPath = path.join(repoPath, '.moatless', 'config.json')
    return await pathExists(configPath)
  }

  /**
   * Read repository configuration from .moatless/config.json file
   * Tries local path first, falls back to GitHub API if remote URL is provided
   * @param repoPath Local path to the repository
   * @param remoteUrl Optional GitHub remote URL
   * @param sessionToken Optional GitHub session token for authentication
   * @returns Parsed configuration file or null if not found/invalid
   */
  async readRepoConfigFile(
    repoPath: string,
    remoteUrl?: string,
    sessionToken?: string,
  ): Promise<RepositoryConfigFile | null> {
    const configPath = path.join(repoPath, '.moatless', 'config.json')

    // Try reading from local path first
    let content = await readFileSafe(configPath)

    // If not found locally and remoteUrl is available, try GitHub API
    if (!content && remoteUrl) {
      const pat = this.getGitHubToken(sessionToken)
      if (pat) {
        try {
          const parsedUrl = this.parseGitUrl(remoteUrl)
          const githubService = getGitHubService()
          content = await githubService.getFileContent(
            pat,
            parsedUrl.owner,
            parsedUrl.repoName,
            '.moatless/config.json',
          )
        } catch (error) {
          logger.debug('Failed to read config file from GitHub', {
            remoteUrl,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      }
    }

    if (!content) {
      return null
    }

    // Parse and validate the config file
    try {
      const json = JSON.parse(content)
      const validated = RepositoryConfigFileSchema.parse(json)
      return validated
    } catch (error) {
      logger.warn('Invalid repository config file format', {
        path: configPath,
        remoteUrl,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }

  /**
   * Write repository configuration to .moatless/config.json file
   * @param repoPath Local path to the repository
   * @param config Configuration to write
   */
  async writeRepoConfigFile(repoPath: string, config: RepositoryConfigFile): Promise<void> {
    const configPath = path.join(repoPath, '.moatless', 'config.json')

    try {
      const content = JSON.stringify(config, null, 2)
      await writeFileSafe(configPath, content)
      logger.info('Created repository config file', { path: configPath })
    } catch (error) {
      logger.error('Failed to write repository config file', error as Error)
      throw new ValidationError('Failed to create .moatless/config.json file', {
        path: configPath,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  /**
   * Merge repository configuration with config file if sync is enabled
   * @param repository Repository configuration
   * @returns Repository with merged config from file if sync is enabled
   */
  async mergeWithRepoConfig(repository: RepositoryConfig): Promise<RepositoryConfig> {
    if (!repository.syncWithRepoConfig) {
      return repository
    }

    const configFile = await this.readRepoConfigFile(
      repository.path,
      repository.remoteUrl,
      undefined,
    )

    if (!configFile) {
      logger.warn('Repository has syncWithRepoConfig enabled but no config file found', {
        repositoryId: repository.id,
        path: repository.path,
      })
      return repository
    }

    // Merge config file values into repository
    return {
      ...repository,
      ...(configFile.systemPrompt !== undefined && { systemPrompt: configFile.systemPrompt }),
      ...(configFile.codeExecution !== undefined && { codeExecution: configFile.codeExecution }),
      ...(configFile.previewConfig !== undefined && { previewConfig: configFile.previewConfig }),
    }
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
          if (onlyGitRepos && !(await isRegularGitRepository(projectPath))) {
            continue
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
}

export function createRepositoryService(storage: StorageBackend): RepositoryService {
  return new RepositoryService(storage)
}
