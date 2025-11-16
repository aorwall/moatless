import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CodeExecution,
  CreateLocalFromTemplateRequest,
  CreateLocalRepositoryRequest,
  CreateRemoteRepositoryRequest,
  CreateRepositoryRequest,
  CreateTemplateRepositoryRequest,
  LocalRepository,
  PreviewConfig,
  RemoteRepository,
  RepositoryConfig,
  TemplateRepository,
  UpdateRepositoryRequest,
} from '@moatless/types'
import { RepositoryNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { getCurrentUserIdForCreation } from '../../../features/auth'
import { GitProviderTokenService } from '../../../features/auth/services/git-provider-token.service'
import type { GitHubProviderClient } from '../../../infrastructure/clients/github-provider.client'
import { GitHubProviderClientSymbol } from '../../../infrastructure/clients/github-provider.client'
import type { GitLabProviderClient } from '../../../infrastructure/clients/gitlab-provider.client'
import { GitLabProviderClientSymbol } from '../../../infrastructure/clients/gitlab-provider.client'
import type { SettingsService } from '../../../services/settings.service'
import { getDataDir } from '../../../utils/data-dir'
import { isRegularGitRepository } from '../../../utils/git-utils'
import { logger } from '../../../utils/logger'
import type { ClaudeService } from '../../claude/index'
import { RepositoryRepository } from '../db/repository.repository'
import { mergeWithRepoConfig } from '../utils/repository-config.utils'
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

  constructor(
    @inject(RepositoryGitService) private readonly gitService: RepositoryGitService,
    @inject(RepositoryRepository) private readonly repository: RepositoryRepository,
    @inject(GitHubProviderClientSymbol) private readonly githubProvider: GitHubProviderClient,
    @inject(GitLabProviderClientSymbol) private readonly gitlabProvider: GitLabProviderClient,
    @inject(GitProviderTokenService)
    private readonly gitProviderTokenService: GitProviderTokenService,
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

  // Setter for GitProviderTokenService (pass through to gitService)
  setGitProviderTokenService(gitProviderTokenService: GitProviderTokenService): void {
    this.gitService.setGitProviderTokenService(gitProviderTokenService)
  }

  /**
   * Create a template repository (source template for creating local repositories)
   * Template repositories are NOT used directly for tasks - they're cloned to create local repos
   */
  async createTemplateRepository(
    request: CreateTemplateRepositoryRequest,
  ): Promise<TemplateRepository> {
    // Validate git URL is accessible (optional - could be skipped for performance)
    // For now, we'll just create the template record without cloning

    const repository: TemplateRepository = {
      kind: 'template',
      id: randomUUID(),
      name: request.name,
      description: request.description,
      remote: {
        url: request.remoteUrl,
        provider:
          request.provider || this.gitProviderTokenService.detectProvider(request.remoteUrl),
        defaultBranch: undefined, // Could be fetched from remote
      },
      codeExecution: request.codeExecution || {},
      previewConfig: request.previewConfig,
      systemPrompt: request.systemPrompt,
      createdBy: getCurrentUserIdForCreation(),
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)
    return repository
  }

  /**
   * Create a local repository from a template repository
   * Clones the template's git repository and creates a new local repository instance
   */
  async createLocalRepositoryFromTemplate(
    request: CreateLocalFromTemplateRequest,
  ): Promise<LocalRepository> {
    // Load template repository from database
    const template = await this.repository.get(request.templateId)
    if (!template || template.kind !== 'template') {
      throw new ValidationError(`Template repository with id ${request.templateId} not found`, {
        templateId: request.templateId,
      })
    }

    const repoDir = this.getRepositoriesDirectory()
    const targetPath = path.join(repoDir, 'from-templates', request.name)

    await fs.mkdir(path.dirname(targetPath), { recursive: true })

    // Clone template repository to target path
    logger.debug('Cloning template repository', {
      templateId: template.id,
      templateName: template.name,
      gitUrl: template.remote.url,
      targetPath,
    })

    const { cloneRepository } = await import('../../../utils/git-utils.js')
    await cloneRepository(template.remote.url, targetPath)

    // Remove remote origin to disconnect from template repository
    await this.executeCommand('git remote remove origin', targetPath).catch(() => {
      // Ignore error if no remote origin exists
    })

    // Add empty commit to mark this as created from template
    await this.executeCommand(
      `git commit --allow-empty -m "Created from template: ${template.name}"`,
      targetPath,
    )

    logger.info('Successfully created local repository from template', {
      templateId: template.id,
      templateName: template.name,
      gitUrl: template.remote.url,
      targetPath,
    })

    // Merge template config with user overrides
    const codeExecution: CodeExecution = {
      ...template.codeExecution,
      ...request.codeExecution,
    }

    const previewConfig: PreviewConfig | undefined = template.previewConfig
      ? {
          ...template.previewConfig,
          ...request.previewConfig,
        }
      : request.previewConfig

    const repository: LocalRepository = {
      kind: 'local',
      id: randomUUID(),
      name: request.name,
      description: request.description || `Created from template: ${template.name}`,
      path: targetPath,
      useGitWorktrees: false,
      codeExecution,
      previewConfig,
      systemPrompt: request.systemPrompt || template.systemPrompt,
      createdBy: getCurrentUserIdForCreation(),
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)
    return repository
  }

  /**
   * Create a local repository from an existing directory
   */
  async createLocalRepository(request: CreateLocalRepositoryRequest): Promise<LocalRepository> {
    // Validate the path exists and is a git repository
    await this.gitService.validateRepository(request.path)

    const repository: LocalRepository = {
      kind: 'local',
      id: randomUUID(),
      name: request.name,
      description: request.description,
      path: request.path,
      useGitWorktrees: request.useGitWorktrees ?? false,
      originalBranch: request.originalBranch,
      codeExecution: request.codeExecution || {},
      previewConfig: request.previewConfig,
      systemPrompt: request.systemPrompt,
      createdBy: getCurrentUserIdForCreation(),
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    // Always merge config from file if present
    const merged = (await this.mergeRepositoryWithConfig(repository)) as LocalRepository

    await this.saveRepository(merged)
    return merged
  }

  /**
   * Create a remote repository (GitHub/GitLab) without cloning
   */
  async createRemoteRepository(
    request: CreateRemoteRepositoryRequest,
    sessionToken?: string,
  ): Promise<RemoteRepository> {
    // Auto-detect provider from URL
    const provider =
      request.provider || this.gitProviderTokenService.detectProvider(request.remoteUrl)

    // Get appropriate token based on provider
    const token = await this.gitProviderTokenService.getTokenForProvider(provider, sessionToken)

    // Get default branch using appropriate provider
    let defaultBranch: string
    if (provider === 'github') {
      defaultBranch = await this.githubProvider.getDefaultBranch(
        request.remoteUrl,
        token || undefined,
      )
    } else if (provider === 'gitlab') {
      defaultBranch = await this.gitlabProvider.getDefaultBranch(
        request.remoteUrl,
        token || undefined,
      )
    } else {
      throw new ValidationError(`Unsupported provider: ${provider}`, { provider })
    }

    logger.info('Creating remote repository', {
      name: request.name,
      remoteUrl: request.remoteUrl,
      provider,
      defaultBranch,
    })

    const repository: RemoteRepository = {
      kind: 'remote',
      id: randomUUID(),
      name: request.name,
      description: request.description,
      remote: {
        url: request.remoteUrl,
        provider,
        defaultBranch,
      },
      codeExecution: request.codeExecution || {},
      previewConfig: request.previewConfig,
      systemPrompt: request.systemPrompt,
      createdBy: getCurrentUserIdForCreation(),
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)
    return repository
  }

  /**
   * Create a new repository (dispatches to specific creation method based on kind)
   */
  async createRepository(
    request: CreateRepositoryRequest,
    sessionToken?: string,
  ): Promise<RepositoryConfig> {
    switch (request.kind) {
      case 'local':
        return await this.createLocalRepository(request)
      case 'remote':
        return await this.createRemoteRepository(request, sessionToken)
      case 'template':
        return await this.createTemplateRepository(request)
      default: {
        const _exhaustiveCheck: never = request
        throw new ValidationError('Invalid repository kind', {
          kind: (_exhaustiveCheck as CreateRepositoryRequest).kind,
        })
      }
    }
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

    // If sync is enabled, merge config from file
    // Note: selectToRepository already constructs valid objects with proper defaults
    return await this.mergeRepositoryWithConfig(repository)
  }

  /**
   * List all repositories from the database (including templates).
   * Filtering by kind should be done client-side for better caching.
   */
  async listRepositories(): Promise<RepositoryConfig[]> {
    const repositories = await this.repository.getAll(false)
    // Sort by creation date (newest first)
    return repositories.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
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

    // Build updated repository
    const updatedRepository = this.buildUpdatedRepository(repoId, repository, updates)

    await this.saveRepository(updatedRepository)

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
   * Convert a remote repository to a template repository
   * Templates cannot be used directly for tasks - only for creating local repositories
   */
  async convertToTemplate(repoId: string): Promise<TemplateRepository> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // Validate it's a remote repository
    if (repository.kind !== 'remote') {
      throw new ValidationError('Only remote repositories can be converted to templates', {
        repositoryId: repoId,
        currentKind: repository.kind,
      })
    }

    // Convert by changing kind from 'remote' to 'template'
    const template: TemplateRepository = {
      ...repository,
      kind: 'template',
      updatedAt: new Date(),
    }

    await this.saveRepository(template)

    logger.info('Repository converted to template', {
      repositoryId: repoId,
      name: repository.name,
      remoteUrl: repository.remote.url,
    })

    return template
  }

  /**
   * Rename a repository (updates display name only, not directory).
   *
   * This method:
   * 1. Validates the repository exists
   * 2. Updates the repository name in the database
   *
   * Note: This only updates the display name. The physical directory path remains unchanged.
   * Duplicate repository names are allowed.
   *
   * @param repoId - Repository ID to rename
   * @param newName - New repository name
   * @throws RepositoryNotFoundError if repository doesn't exist
   */
  async renameRepository(repoId: string, newName: string): Promise<void> {
    logger.info('Starting repository rename', { repoId, newName })

    // 1. Get repository
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // 2. Early return if name hasn't changed
    if (repository.name === newName) {
      logger.info('Repository name unchanged, skipping rename', { repoId, name: newName })
      return
    }

    // 3. Update repository name in database (path remains unchanged)
    repository.name = newName
    await this.saveRepository(repository)

    const logContext: Record<string, unknown> = { repoId, newName }
    if (repository.kind === 'local') {
      logContext['path'] = repository.path
    } else if (repository.kind === 'remote' || repository.kind === 'template') {
      logContext['remoteUrl'] = repository.remote.url
    }
    logger.info('Repository renamed successfully', logContext)
  }

  /**
   * Get branches for a repository
   */
  async getBranches(repositoryId: string): Promise<string[]> {
    const repository = await this.getRepositoryById(repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError(repositoryId)
    }

    switch (repository.kind) {
      case 'local':
        // Use local git commands
        return await this.gitService.getBranches(repository.path)

      case 'remote':
      case 'template': {
        // Use provider API (no local clone needed)
        const provider = repository.remote.provider
        const token = await this.gitProviderTokenService.getTokenForProvider(provider)

        let branches: string[]
        if (provider === 'github') {
          branches = await this.githubProvider.listBranches(
            repository.remote.url,
            token || undefined,
          )
        } else if (provider === 'gitlab') {
          branches = await this.gitlabProvider.listBranches(
            repository.remote.url,
            token || undefined,
          )
        } else {
          throw new ValidationError(`Unsupported provider: ${provider}`, { provider })
        }

        // Sort branches with default branch first
        return this.sortBranchesWithDefault(branches, repository.remote.defaultBranch)
      }

      default: {
        const _exhaustiveCheck: never = repository
        throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
      }
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

  // Private helper methods

  /**
   * Sort branches with the default branch first, then alphabetically
   */
  private sortBranchesWithDefault(branches: string[], defaultBranch?: string): string[] {
    return branches.sort((a, b) => {
      // Prioritize the default branch
      if (defaultBranch) {
        if (a === defaultBranch) {
          return -1
        }
        if (b === defaultBranch) {
          return 1
        }
      }

      // Alphabetical for the rest
      return a.localeCompare(b)
    })
  }

  private getRepositoriesDirectory(): string {
    return (
      this.settingsService?.getRepositoriesDir() ||
      process.env['REPO_DIR'] ||
      path.join(getDataDir(), 'repos')
    )
  }

  private buildUpdatedRepository(
    repoId: string,
    repository: RepositoryConfig,
    updates: UpdateRepositoryRequest,
  ): RepositoryConfig {
    // Build base updates (common to all kinds)
    const baseUpdates = {
      id: repoId,
      name: updates.name ?? repository.name,
      description:
        updates.description !== undefined
          ? (updates.description ?? undefined)
          : repository.description,
      codeExecution: updates.codeExecution ?? repository.codeExecution,
      previewConfig: updates.previewConfig ?? repository.previewConfig,
      systemPrompt:
        updates.systemPrompt !== undefined
          ? (updates.systemPrompt ?? undefined)
          : repository.systemPrompt,
      deleted: repository.deleted,
      createdAt: repository.createdAt,
      updatedAt: new Date(),
    }

    // Handle each repository kind separately
    switch (repository.kind) {
      case 'local': {
        const localUpdates: LocalRepository = {
          ...baseUpdates,
          kind: 'local',
          path: updates.path ?? repository.path,
          useGitWorktrees: updates.useGitWorktrees ?? repository.useGitWorktrees,
          originalBranch:
            updates.originalBranch !== undefined
              ? (updates.originalBranch ?? undefined)
              : repository.originalBranch,
        }
        return localUpdates
      }

      case 'remote': {
        const remoteUpdates: RemoteRepository = {
          ...baseUpdates,
          kind: 'remote',
          remote: repository.remote,
        }
        return remoteUpdates
      }

      case 'template': {
        const templateUpdates: TemplateRepository = {
          ...baseUpdates,
          kind: 'template',
          remote: {
            url: updates.remoteUrl ?? repository.remote.url,
            provider: updates.provider ?? repository.remote.provider,
            defaultBranch:
              updates.defaultBranch !== undefined
                ? (updates.defaultBranch ?? undefined)
                : repository.remote.defaultBranch,
          },
        }
        return templateUpdates
      }

      default: {
        const _exhaustiveCheck: never = repository
        throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
      }
    }
  }

  private async mergeRepositoryWithConfig(repository: RepositoryConfig): Promise<RepositoryConfig> {
    // Get remoteUrl for remote repositories
    const remoteUrl = repository.kind === 'remote' ? repository.remote.url : undefined

    // Only get GitHub token if this is a GitHub repository (for config file reading)
    let githubToken: string | null = null
    if (remoteUrl?.includes('github.com')) {
      githubToken = await this.gitProviderTokenService.getTokenForProvider('github')
    }

    return await mergeWithRepoConfig(repository, {
      ...(remoteUrl && { remoteUrl }),
      ...(githubToken && { githubToken }),
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
