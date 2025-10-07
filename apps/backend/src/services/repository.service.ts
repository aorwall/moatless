import type { ChildProcess } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CodeExecution,
  CommandExecutionOptions,
  PreviewConfig,
  ShellCommandLogEvent,
  WorkspaceStatusResponse,
} from '@moatless/types'
import {
  type CreateRepositoryRequest,
  type RepositoryConfig,
  RepositoryConfigSchema,
  RepositoryNotFoundError,
  type RepositoryTemplate,
  ValidationError,
} from '@moatless/types'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import { TEMPLATES } from '../config/templates.js'
import { getDataDir } from '../utils/data-dir.js'
import { cloneRepository, isRegularGitRepository } from '../utils/git-utils.js'
import { searchFilesInDirectoryNew } from '../utils/index.js'
import { logger } from '../utils/logger.js'
import type { Workspace } from '../workspaces/workspace.js'
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

  private async createFromLegacyTemplate(templatePath: string, finalName: string): Promise<string> {
    const repoDir = this.getRepositoriesDirectory()
    const targetPath = path.join(repoDir, finalName)
    const templateName = path.basename(templatePath)

    await fs.mkdir(repoDir, { recursive: true })
    await this.forkFromTemplate(templatePath, targetPath, templateName)

    return targetPath
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

    if (!request.path && request.templatePath) {
      const targetPath = await this.createFromLegacyTemplate(request.templatePath, request.name)
      return { path: targetPath, name: request.name }
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
      `git commit --allow-empty -m "Forked from template: ${template.name}"`,
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

    const repository: RepositoryConfig = {
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
      codeExecution: request.codeExecution,
      previewConfig: request.previewConfig,
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
    // Parse through schema to ensure legacy repositories get default values
    if (repository) {
      return RepositoryConfigSchema.parse(repository)
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

  async updateRepository(
    repoId: string,
    updates: {
      [K in keyof RepositoryConfig]?: RepositoryConfig[K] | undefined
    },
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

    // Build updated repository, only including defined update values
    const updatedRepository: RepositoryConfig = {
      ...repository,
      ...(updates.name !== undefined && { name: updates.name }),
      ...(updates.path !== undefined && { path: updates.path }),
      ...(updates.description !== undefined && {
        description: updates.description,
      }),
      ...(updates.remoteUrl !== undefined && { remoteUrl: updates.remoteUrl }),
      ...(updates.sourceType !== undefined && {
        sourceType: updates.sourceType,
      }),
      ...(updates.useGitWorktrees !== undefined && {
        useGitWorktrees: updates.useGitWorktrees,
      }),
      ...(updates.cloneOnNewTask !== undefined && {
        cloneOnNewTask: updates.cloneOnNewTask,
      }),
      ...(updates.codeExecution !== undefined && {
        codeExecution: updates.codeExecution,
      }),
      ...(updates.systemPrompt !== undefined && {
        systemPrompt: updates.systemPrompt,
      }),
      ...(updates.originalBranch !== undefined && {
        originalBranch: updates.originalBranch,
      }),
      ...(updates.previewConfig !== undefined && {
        previewConfig: updates.previewConfig,
      }),
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

  async searchFiles(repoId: string, request: { query: string; limit?: number }) {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    const result = await searchFilesInDirectoryNew(repository.path, request)

    // Transform to match RepositoryFileSearchResponseSchema
    return {
      files: result.results.map((r) => ({
        path: r.path,
        type: 'file',
        size: undefined,
      })),
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
   * Clone a Git repository (GitHub or other Git hosting services)
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

    const { clonePath, repoDir } = await this.prepareCloneDirectory(finalName)
    const cloneUrl = this.buildCloneUrl(remoteUrl, parsedUrl, pat)

    this.logCloneStart(parsedUrl, clonePath, pat)

    await this.executeCloneWithSSH(
      cloneUrl,
      clonePath,
      parsedUrl.isSSH,
      repoDir,
      parsedUrl,
      remoteUrl,
      pat,
    )

    if (parsedUrl.isGitHub && !parsedUrl.isSSH && pat) {
      await this.removePATFromRemote(remoteUrl, clonePath)
    }

    logger.info('Successfully cloned Git repository', {
      hostname: parsedUrl.hostname,
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
    hostname: string | undefined
    isSSH: boolean
    isGitHub: boolean
  } {
    const githubHttpsPattern = /(?:https?:\/\/)?(?:www\.)?github\.com\/([^/]+)\/([^/.]+)(?:\.git)?$/
    const githubSshPattern = /^git@github\.com:([^/]+)\/([^/.]+?)(?:\.git)?$/
    const genericGitSshPattern = /^git@([^:]+):(?:([^/]+)\/)?([^/.]+?)(?:\.git)?$/

    let match = remoteUrl.match(githubHttpsPattern)
    if (match) {
      return {
        owner: match[1] || '',
        repoName: match[2] || '',
        hostname: 'github.com',
        isSSH: false,
        isGitHub: true,
      }
    }

    match = remoteUrl.match(githubSshPattern)
    if (match) {
      return {
        owner: match[1] || '',
        repoName: match[2] || '',
        hostname: 'github.com',
        isSSH: true,
        isGitHub: true,
      }
    }

    match = remoteUrl.match(genericGitSshPattern)
    if (match) {
      return {
        owner: match[2] || '',
        repoName: match[3] || '',
        hostname: match[1] || '',
        isSSH: true,
        isGitHub: false,
      }
    }

    throw new ValidationError('Invalid Git URL format', { url: remoteUrl })
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

  private buildCloneUrl(
    remoteUrl: string,
    parsedUrl: { isSSH: boolean; isGitHub: boolean },
    pat: string | undefined,
  ): string {
    if (parsedUrl.isSSH || !parsedUrl.isGitHub || !pat) {
      return remoteUrl
    }

    return remoteUrl.replace(/^https:\/\/github\.com/, `https://${pat}@github.com`)
  }

  private logCloneStart(
    parsedUrl: {
      hostname: string | undefined
      owner: string
      repoName: string
      isSSH: boolean
      isGitHub: boolean
    },
    clonePath: string,
    pat: string | undefined,
  ): void {
    logger.info('Cloning Git repository', {
      hostname: parsedUrl.hostname,
      owner: parsedUrl.owner,
      repo: parsedUrl.repoName,
      path: clonePath,
      isSSH: parsedUrl.isSSH,
      isGitHub: parsedUrl.isGitHub,
      hasAuth: parsedUrl.isGitHub && !parsedUrl.isSSH && Boolean(pat),
    })
  }

  private async buildSSHEnvironment(
    isSSH: boolean,
    repoDir: string,
  ): Promise<Record<string, string>> {
    if (!isSSH) {
      return {}
    }

    const sshKeyPath = await this.getSSHKeyPathIfNeeded(repoDir)
    if (!sshKeyPath) {
      return {}
    }

    return {
      GIT_SSH_COMMAND: `ssh -i ${sshKeyPath} -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no`,
    }
  }

  private async getSSHKeyPathIfNeeded(repoDir: string): Promise<string | null> {
    const testResult = await this.executeCommand('cat ~/.ssh/known_hosts 2>&1', repoDir)

    if (testResult.success && !testResult.error?.includes('Permission denied')) {
      logger.info('SSH permissions OK, proceeding with normal clone')
      return null
    }

    const dataDir = getDataDir()
    const serviceSSHDir = `${path.dirname(dataDir)}/.ssh`
    const sshKeyPath = await this.findSSHKey(serviceSSHDir, repoDir)

    logger.info('Using service SSH key for systemd environment', { sshKeyPath })
    return sshKeyPath
  }

  private async findSSHKey(serviceSSHDir: string, repoDir: string): Promise<string> {
    let sshKeyPath = `${serviceSSHDir}/id_ed25519`
    const keyCheckResult = await this.executeCommand(
      `test -f ${sshKeyPath} && echo "exists" || echo "not found"`,
      repoDir,
    )

    if (keyCheckResult.output?.includes('not found')) {
      sshKeyPath = `${serviceSSHDir}/id_rsa`
    }

    return sshKeyPath
  }

  private async executeCloneWithSSH(
    cloneUrl: string,
    clonePath: string,
    isSSH: boolean,
    repoDir: string,
    parsedUrl: { isSSH: boolean; isGitHub: boolean; hostname: string | undefined },
    remoteUrl: string,
    pat: string | undefined,
  ): Promise<void> {
    try {
      // Set SSH environment if needed
      const sshEnv = await this.buildSSHEnvironment(isSSH, repoDir)

      // Temporarily set environment variables for SSH
      const originalEnv: Record<string, string | undefined> = {}
      for (const [key, value] of Object.entries(sshEnv)) {
        originalEnv[key] = process.env[key]
        process.env[key] = value
      }

      try {
        // Use the git-utils cloneRepository function
        await cloneRepository(cloneUrl, clonePath)
      } finally {
        // Restore original environment
        for (const [key, originalValue] of Object.entries(originalEnv)) {
          if (originalValue === undefined) {
            Reflect.deleteProperty(process.env, key)
          } else {
            process.env[key] = originalValue
          }
        }
      }
    } catch (error) {
      logger.error('Git clone failed', error as Error, {
        isSSH: parsedUrl.isSSH,
        isGitHub: parsedUrl.isGitHub,
      })

      const errorMessage = this.buildCloneErrorMessage(
        error instanceof Error ? error.message : String(error),
        parsedUrl,
        pat,
      )

      throw new ValidationError(errorMessage, {
        url: remoteUrl,
        error: (error instanceof Error ? error.message : String(error)).replace(pat || '', '***'),
        isSSH: parsedUrl.isSSH,
        isGitHub: parsedUrl.isGitHub,
      })
    }
  }

  private buildCloneErrorMessage(
    error: string | undefined,
    parsedUrl: { isSSH: boolean; isGitHub: boolean; hostname: string | undefined },
    pat: string | undefined,
  ): string {
    const sanitizedError = error?.replace(pat || '', '***')
    let message = `Failed to clone repository: ${sanitizedError}`

    if (parsedUrl.isSSH) {
      message += `\n\nFor SSH URLs, ensure you have SSH keys configured for ${parsedUrl.hostname || 'the Git host'}.`
    } else if (parsedUrl.isGitHub && !pat) {
      message +=
        '\n\nFor GitHub HTTPS URLs, consider adding a GitHub Personal Access Token in settings.'
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
   * Fork from template preserving git history and removing remote origin
   */
  private async forkFromTemplate(src: string, dest: string, templateName: string): Promise<void> {
    await fs.mkdir(dest, { recursive: true })
    const entries = await fs.readdir(src, { withFileTypes: true })

    for (const entry of entries) {
      const srcPath = path.join(src, entry.name)
      const destPath = path.join(dest, entry.name)

      if (entry.isDirectory() && entry.name === 'node_modules') {
        // Skip only node_modules, preserve .git for forking
        continue
      }

      if (entry.isDirectory()) {
        await this.forkFromTemplate(srcPath, destPath, templateName)
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

    // Only perform git operations on the root level (when dest contains .git)
    const gitPath = path.join(dest, '.git')
    try {
      await fs.access(gitPath)
      // Git repository exists, remove remote origin and add fork commit
      await this.executeCommand('git remote remove origin', dest).catch(() => {
        // Ignore error if no remote origin exists
      })
      await this.executeCommand(
        `git commit --allow-empty -m "Forked from template: ${templateName}"`,
        dest,
      )
      logger.info('Successfully forked from template with preserved git history', {
        templateName,
        dest,
      })
    } catch {
      // No .git directory found at this level, continue
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

  /**
   * Verify repository setup by creating a temporary workspace and running setup commands
   */
  /**
   * Execute commands in a repository workspace with streaming output
   */
  async *executeCommands(
    repoId: string,
    options: CommandExecutionOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    if (!this.workspaceService) {
      throw new Error('WorkspaceService not available for command execution')
    }

    logger.info('Executing commands in repository', {
      repositoryId: repoId,
      purpose: options.purpose,
      commandCount: options.commands.length,
    })

    // Use workspace service to execute commands in a temporary workspace
    for await (const event of this.workspaceService.executeCommandsInTemporaryWorkspace(
      repoId,
      options,
    )) {
      yield event
    }
  }

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
    let workspace: Workspace | null = null

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
        const currentStatus = workspaceStatus.environmentStatus

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
          const error = workspaceStatus.environmentError || 'Unknown error'
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
