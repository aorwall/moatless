import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CodeExecution,
  CreateRemoteFromTemplateRequest,
  CreateRemoteRepositoryRequest,
  CreateRepositoryRequest,
  CreateTemplateRepositoryRequest,
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
import { logger } from '../../../utils/logger'
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
  private settingsService?: SettingsService

  constructor(
    @inject(RepositoryGitService) private readonly gitService: RepositoryGitService,
    @inject(RepositoryRepository) private readonly repository: RepositoryRepository,
    @inject(GitHubProviderClientSymbol) private readonly githubProvider: GitHubProviderClient,
    @inject(GitLabProviderClientSymbol) private readonly gitlabProvider: GitLabProviderClient,
    @inject(GitProviderTokenService)
    private readonly gitProviderTokenService: GitProviderTokenService,
  ) {}

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
   * Create a template repository (source template for creating repositories)
   * Template repositories are NOT used directly for tasks - they're used to create new repos
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
      visibility: request.visibility ?? 'private',
      createdBy: getCurrentUserIdForCreation(),
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)
    return repository
  }

  /**
   * Create a remote repository (GitHub) from a template repository
   *
   * Workflow:
   * 1. Load template repository
   * 2. Create GitHub repository
   * 3. Clone template to temp directory
   * 4. Push to GitHub
   * 5. Clean up temp directory
   * 6. Save and return RemoteRepository record
   */
  async createRemoteRepositoryFromTemplate(
    request: CreateRemoteFromTemplateRequest,
    sessionToken?: string,
  ): Promise<RemoteRepository> {
    // 1. Load template repository from database
    const template = await this.repository.get(request.templateId)
    if (!template || template.kind !== 'template') {
      throw new ValidationError(`Template repository with id ${request.templateId} not found`, {
        templateId: request.templateId,
      })
    }

    // 2. Get GitHub token
    const token = await this.gitProviderTokenService.getTokenForProvider('github', sessionToken)
    if (!token) {
      throw new ValidationError(
        'GitHub authentication required to create a repository. Please connect your GitHub account in Settings.',
        { templateId: request.templateId },
      )
    }

    // 3. Create GitHub repository
    logger.info(
      {
        templateId: template.id,
        templateName: template.name,
        repositoryName: request.name,
        visibility: request.visibility ?? 'private',
      },
      'Creating GitHub repository from template',
    )

    const githubResult = await this.githubProvider.createRepository(
      token,
      request.name,
      (request.visibility ?? 'private') === 'private',
    )

    logger.info(
      {
        templateId: template.id,
        githubUrl: githubResult.url,
        cloneUrl: githubResult.cloneUrl,
      },
      'GitHub repository created successfully',
    )

    // 4. Clone template to temp directory and push to GitHub
    const tempDir = path.join(this.getRepositoriesDirectory(), 'temp', `fork-${randomUUID()}`)
    await fs.mkdir(tempDir, { recursive: true })

    try {
      // Clone template
      const { cloneRepository } = await import('../../../utils/git-utils.js')
      await cloneRepository(template.remote.url, tempDir)

      // Remove original origin
      await this.executeCommand('git remote remove origin', tempDir).catch(() => {
        // Ignore error if no remote origin exists
      })

      // Add new origin with authentication
      const authenticatedUrl = this.gitProviderTokenService.buildAuthenticatedCloneUrl(
        githubResult.cloneUrl,
        token,
        'github',
      )
      await this.executeCommand(`git remote add origin ${authenticatedUrl}`, tempDir)

      // Push all branches
      const pushResult = (await this.executeCommand('git push -u origin --all', tempDir)) as {
        success: boolean
        error?: string
      }
      if (!pushResult.success) {
        throw new Error(`Failed to push to GitHub: ${pushResult.error}`)
      }

      // Push tags if any
      await this.executeCommand('git push origin --tags', tempDir).catch(() => {
        // Ignore errors when pushing tags
      })

      logger.info(
        { templateId: template.id, githubUrl: githubResult.url },
        'Code pushed to GitHub successfully',
      )
    } finally {
      // 5. Clean up temp directory
      await fs.rm(tempDir, { recursive: true, force: true }).catch((err) => {
        logger.warn({ tempDir, err }, 'Failed to clean up temp directory')
      })
    }

    // 6. Merge template config with user overrides
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

    // 7. Create and save RemoteRepository record
    const repository: RemoteRepository = {
      kind: 'remote',
      id: randomUUID(),
      name: request.name,
      description: request.description || `Created from template: ${template.name}`,
      remote: {
        url: githubResult.url,
        provider: 'github',
        defaultBranch: githubResult.defaultBranch,
      },
      codeExecution,
      previewConfig,
      systemPrompt: request.systemPrompt || template.systemPrompt,
      visibility: request.visibility ?? 'private',
      createdBy: getCurrentUserIdForCreation(),
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)

    logger.info(
      {
        templateId: template.id,
        templateName: template.name,
        repositoryId: repository.id,
        githubUrl: githubResult.url,
      },
      'Successfully created remote repository from template',
    )

    return repository
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

    logger.info(
      {
        name: request.name,
        remoteUrl: request.remoteUrl,
        provider,
        defaultBranch,
      },
      'Creating remote repository',
    )

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
      visibility: request.visibility ?? 'private',
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
   * Get a repository by ID without access control (for internal/system operations).
   * Use this for background tasks like event processing where there's no user session context.
   */
  async getRepositoryByIdInternal(repoId: string): Promise<RepositoryConfig | null> {
    const repository = await this.repository.getInternal(repoId)
    if (!repository) {
      return null
    }

    return await this.mergeRepositoryWithConfig(repository)
  }

  /**
   * List all repositories from the database (including templates).
   * Filtering by kind should be done client-side for better caching.
   */
  async listRepositories(): Promise<RepositoryConfig[]> {
    const repositories = await this.repository.getAll(false)
    // Sort alphabetically by name (A-Z)
    return repositories.sort((a, b) => a.name.localeCompare(b.name))
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
   * Templates cannot be used directly for tasks - only for creating new repositories
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

    logger.info(
      {
        repositoryId: repoId,
        name: repository.name,
        remoteUrl: repository.remote.url,
      },
      'Repository converted to template',
    )

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
    logger.info({ repoId, newName }, 'Starting repository rename')

    // 1. Get repository
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // 2. Early return if name hasn't changed
    if (repository.name === newName) {
      logger.info({ repoId, name: newName }, 'Repository name unchanged, skipping rename')
      return
    }

    // 3. Update repository name in database
    repository.name = newName
    await this.saveRepository(repository)

    logger.info(
      { repoId, newName, remoteUrl: repository.remote.url },
      'Repository renamed successfully',
    )
  }

  /**
   * Get branches for a repository
   */
  async getBranches(repositoryId: string): Promise<string[]> {
    const repository = await this.getRepositoryById(repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError(repositoryId)
    }

    // Use provider API (no local clone needed)
    const provider = repository.remote.provider
    const token = await this.gitProviderTokenService.getTokenForProvider(provider)

    let branches: string[]
    if (provider === 'github') {
      branches = await this.githubProvider.listBranches(repository.remote.url, token || undefined)
    } else if (provider === 'gitlab') {
      branches = await this.gitlabProvider.listBranches(repository.remote.url, token || undefined)
    } else {
      throw new ValidationError(`Unsupported provider: ${provider}`, { provider })
    }

    // Ensure default branch is always included (providers limit to 100 branches)
    const defaultBranch = repository.remote.defaultBranch
    if (defaultBranch && !branches.includes(defaultBranch)) {
      branches.unshift(defaultBranch)
    }

    // Sort branches with default branch first
    return this.sortBranchesWithDefault(branches, defaultBranch)
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
      visibility: updates.visibility ?? repository.visibility,
      deleted: repository.deleted,
      createdBy: repository.createdBy,
      createdAt: repository.createdAt,
      updatedAt: new Date(),
    }

    // Handle each repository kind separately
    switch (repository.kind) {
      case 'remote': {
        const remoteUpdates: RemoteRepository = {
          ...baseUpdates,
          kind: 'remote',
          remote: {
            url: updates.remoteUrl ?? repository.remote.url,
            provider: updates.provider ?? repository.remote.provider,
            defaultBranch:
              updates.defaultBranch !== undefined
                ? (updates.defaultBranch ?? undefined)
                : repository.remote.defaultBranch,
          },
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
    // Get remoteUrl for repositories (both remote and template have remote.url)
    const remoteUrl = repository.remote.url

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
