import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CodeExecution,
  CreateRepositoryRequest,
  PreviewConfig,
  RepositoryConfig,
  UpdateRepositoryRequest,
} from '@moatless/types'
import { RepositoryConfigSchema, RepositoryNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { TEMPLATES } from '../../../config/templates'
import type { GitHubService } from '../../../services/github.service'
import type { SettingsService } from '../../../services/settings.service'
import { getDataDir } from '../../../utils/data-dir'
import { isRegularGitRepository } from '../../../utils/git-utils'
import { logger } from '../../../utils/logger'
import type { ClaudeService } from '../../agents/index'
import { RepositoryRepository } from '../db/repository.repository'
import {
  detectRepoConfigFile,
  mergeWithRepoConfig,
  syncConfigToFile,
} from '../utils/repository-config.utils'
import { RepositoryGitService } from './repository-git.service'

/**
 * Main repository service handling business logic and orchestration.
 *
 * Uses Drizzle ORM-based RepositoryRepository for all persistence operations.
 */
@injectable()
export class RepositoryService {
  private claudeService?: ClaudeService
  private settingsService?: SettingsService
  private githubService?: GitHubService

  constructor(
    @inject(RepositoryGitService) private readonly gitService: RepositoryGitService,
    @inject(RepositoryRepository) private readonly repository: RepositoryRepository,
  ) {}

  // Setter for ClaudeService (temporary until full DI refactor)
  setClaudeService(claudeService: ClaudeService): void {
    this.claudeService = claudeService
  }

  // Setter for SettingsService (temporary until full DI refactor)
  setSettingsService(settingsService: SettingsService): void {
    this.settingsService = settingsService
    this.gitService.setSettingsService(settingsService)
  }

  // Setter for GitHubService (temporary until full DI refactor)
  setGitHubService(githubService: GitHubService): void {
    this.githubService = githubService
    this.gitService.setGitHubService(githubService)
  }

  /**
   * Create a repository from a template by cloning it fresh
   */
  async createRepositoryFromTemplate(
    repoName: string,
    templateId: string,
  ): Promise<RepositoryConfig> {
    const template = TEMPLATES.find((t) => t.id === templateId)
    if (!template) {
      throw new ValidationError(`Template with id ${templateId} not found`, {
        templateId,
      })
    }

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

    const { cloneRepository } = await import('../../../utils/git-utils.js')
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

  /**
   * Create a new repository
   */
  async createRepository(
    request: CreateRepositoryRequest,
    sessionToken?: string,
  ): Promise<RepositoryConfig> {
    const resolved = await this.resolveRepositoryPath(request, sessionToken)

    await this.gitService.validateRepository(resolved.path)

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
      const hasConfigFile = await detectRepoConfigFile(resolved.path)
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
      repository = await this.mergeRepositoryWithConfig(repository)
    }

    await this.saveRepository(repository)
    return repository
  }

  /**
   * Save a repository to the database.
   */
  async saveRepository(repo: RepositoryConfig): Promise<void> {
    repo.updatedAt = new Date()
    await this.repository.save(repo)
  }

  /**
   * Get a repository by ID from the database.
   */
  async getRepositoryById(repoId: string): Promise<RepositoryConfig | null> {
    const repository = await this.repository.get(repoId)
    if (!repository) {
      return null
    }

    // Parse through schema to ensure repositories get default values
    const parsed = RepositoryConfigSchema.parse(repository)
    // If sync is enabled, merge config from file
    return await this.mergeRepositoryWithConfig(parsed)
  }

  /**
   * List all repositories from the database.
   */
  async listRepositories(includeVibeRepos = false): Promise<RepositoryConfig[]> {
    const repositories = await this.repository.getAll(false)
    // Filter out vibe repositories unless explicitly requested
    const filtered = repositories.filter((repo) => !repo.vibe || includeVibeRepos)
    // Sort by creation date (newest first)
    return filtered.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
  }

  /**
   * Update a repository
   */
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
      await this.gitService.validateRepository(updates.path)
    }

    // Validate cloneOnNewTask requires remoteUrl
    this.validateCloneSettings(updates, repository)

    // Handle sync config toggle
    this.handleSyncConfigToggle(repoId, repository, updates)

    // Build updated repository
    const updatedRepository = this.buildUpdatedRepository(repoId, repository, updates)

    await this.saveRepository(updatedRepository)

    // Sync config to file if enabled
    await syncConfigToFile(updatedRepository)

    return updatedRepository
  }

  /**
   * Delete a repository (soft delete)
   */
  async deleteRepository(repoId: string): Promise<void> {
    // Use the database repository to perform soft delete
    await this.repository.delete(repoId)
  }

  /**
   * Get branches for a repository
   */
  async getBranches(repoPath: string): Promise<string[]> {
    return await this.gitService.getBranches(repoPath)
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

  // Private helper methods

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
      const cloneResult = await this.gitService.cloneGitRepository(
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

  private async mergeRepositoryWithConfig(repository: RepositoryConfig): Promise<RepositoryConfig> {
    const githubToken = this.githubService
      ? await this.githubService.getTokenForGitOperations()
      : null
    return await mergeWithRepoConfig(repository, {
      ...(repository.remoteUrl && { remoteUrl: repository.remoteUrl }),
      ...(githubToken && { githubToken }),
      ...(this.githubService && { githubService: this.githubService }),
      parseGitUrl: this.gitService.parseGitUrl.bind(this.gitService),
    })
  }

  private async executeCommand(command: string, cwd: string): Promise<unknown> {
    const { spawn } = await import('node:child_process')
    return new Promise((resolve) => {
      const [cmd, ...args] = command.split(' ')
      if (!cmd) {
        return resolve({ success: false, exit_code: 1, error: 'Invalid command' })
      }
      const child = spawn(cmd, args, { cwd, stdio: ['pipe', 'pipe', 'pipe'], shell: true })
      let stdout = ''
      let stderr = ''
      child.stdout?.on('data', (data: Buffer) => {
        stdout += data.toString()
      })
      child.stderr?.on('data', (data: Buffer) => {
        stderr += data.toString()
      })
      child.on('close', (code: number | null) => {
        resolve({
          command,
          success: code === 0,
          exit_code: code || 0,
          output: stdout.trim() || undefined,
          error: stderr.trim() || undefined,
        })
      })
      child.on('error', (error) => {
        resolve({ command, success: false, exit_code: 1, error: error.message })
      })
    })
  }
}
