import type { ChildProcess } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { ValidationError } from '@moatless/types'
import { injectable } from 'inversify'
import type { GitHubTokenService } from '../../../features/auth/services/github-token.service'
import type { GitLabTokenService } from '../../../features/auth/services/gitlab-token.service'
import type { SettingsService } from '../../../services/settings.service'
import { getDataDir } from '../../../utils/data-dir'
import { cloneRepository } from '../../../utils/git-utils'
import { logger } from '../../../utils/logger'
import {
  generateUniqueClonePath,
  parseGitUrl,
  validateLocalRepository,
} from '../utils/repository-validation.utils'

/**
 * Service responsible for git-related repository operations
 */
@injectable()
export class RepositoryGitService {
  private settingsService?: SettingsService
  private githubTokenService?: GitHubTokenService
  private gitlabTokenService?: GitLabTokenService

  // Setter for SettingsService (temporary until full DI refactor)
  setSettingsService(settingsService: SettingsService): void {
    this.settingsService = settingsService
  }

  // Setter for GitHubTokenService (temporary until full DI refactor)
  setGitHubTokenService(githubTokenService: GitHubTokenService): void {
    this.githubTokenService = githubTokenService
  }

  // Setter for GitLabTokenService (temporary until full DI refactor)
  setGitLabTokenService(gitlabTokenService: GitLabTokenService): void {
    this.gitlabTokenService = gitlabTokenService
  }

  /**
   * Detect Git provider from repository URL
   */
  private detectProvider(remoteUrl: string): 'github' | 'gitlab' | 'other' {
    if (remoteUrl.includes('github.com')) {
      return 'github'
    }
    if (remoteUrl.includes('gitlab.com')) {
      return 'gitlab'
    }
    return 'other'
  }

  /**
   * Get branches from a git repository
   */
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

  /**
   * Validate that a repository path exists and is a valid git repository
   */
  async validateRepository(repoPath: string): Promise<void> {
    await validateLocalRepository(repoPath)
  }

  /**
   * Clone a Git repository (GitHub/GitLab) via HTTPS
   */
  async cloneGitRepository(
    remoteUrl: string,
    suggestedName?: string,
    sessionToken?: string,
  ): Promise<{ path: string; name: string }> {
    if (!this.settingsService) {
      throw new Error('SettingsService not available')
    }

    const parsedUrl = parseGitUrl(remoteUrl)
    const finalName = suggestedName || parsedUrl.repoName
    const provider = this.detectProvider(remoteUrl)

    // Get token based on provider
    const pat = await this.getTokenForProvider(provider, sessionToken)

    const { clonePath } = await this.prepareCloneDirectory(finalName)
    const cloneUrl = this.buildCloneUrl(remoteUrl, pat, provider)

    this.logCloneStart(parsedUrl, clonePath, pat, provider)

    await this.executeClone(cloneUrl, clonePath, remoteUrl, pat)

    if (pat) {
      await this.removePATFromRemote(remoteUrl, clonePath)
    }

    logger.info('Successfully cloned repository', {
      provider,
      owner: parsedUrl.owner,
      repo: parsedUrl.repoName,
      name: finalName,
      path: clonePath,
    })

    return { path: clonePath, name: finalName }
  }

  /**
   * Parse git URL to extract owner and repo name
   */
  parseGitUrl(remoteUrl: string): { owner: string; repoName: string } {
    return parseGitUrl(remoteUrl)
  }

  private async getGitHubToken(_sessionToken?: string): Promise<string | null> {
    // Use GitHub token service to get token via unified priority resolution
    // This will check for GitHub OAuth session token or linked provider token
    if (!this.githubTokenService) {
      return null
    }
    return await this.githubTokenService.getTokenForGitOperations()
  }

  private async getTokenForProvider(
    provider: 'github' | 'gitlab' | 'other',
    sessionToken?: string,
  ): Promise<string | null> {
    if (provider === 'github') {
      return await this.getGitHubToken(sessionToken)
    }
    if (provider === 'gitlab') {
      if (!this.gitlabTokenService) {
        return null
      }
      return await this.gitlabTokenService.getTokenForGitOperations()
    }
    return null
  }

  private async prepareCloneDirectory(
    finalName: string,
  ): Promise<{ clonePath: string; repoDir: string }> {
    const repoDir =
      this.settingsService?.getRepositoriesDir() ||
      process.env['REPO_DIR'] ||
      path.join(getDataDir(), 'repos')

    await fs.mkdir(repoDir, { recursive: true })

    const uniqueCloneInfo = await generateUniqueClonePath(repoDir, finalName)
    return { clonePath: uniqueCloneInfo.path, repoDir }
  }

  private buildCloneUrl(
    remoteUrl: string,
    pat: string | null,
    provider: 'github' | 'gitlab' | 'other',
  ): string {
    if (!pat) {
      return remoteUrl
    }

    if (provider === 'github') {
      return remoteUrl.replace(/^https:\/\/github\.com/, `https://${pat}@github.com`)
    }
    if (provider === 'gitlab') {
      return remoteUrl.replace(/^https:\/\/gitlab\.com/, `https://${pat}@gitlab.com`)
    }
    // For other providers, return as-is
    return remoteUrl
  }

  private logCloneStart(
    parsedUrl: {
      owner: string
      repoName: string
    },
    clonePath: string,
    pat: string | null,
    provider: 'github' | 'gitlab' | 'other',
  ): void {
    logger.info('Cloning repository', {
      provider,
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
    pat: string | null,
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

  private buildCloneErrorMessage(error: string | undefined, pat: string | null): string {
    const sanitizedError = error?.replace(pat || '', '***')
    let message = `Failed to clone repository: ${sanitizedError}`

    if (!pat) {
      message +=
        '\n\nConsider setting up a GitHub service connection for authentication (via GitHub OAuth or Personal Access Token).'
    }

    return message
  }

  private async removePATFromRemote(remoteUrl: string, clonePath: string): Promise<void> {
    await this.executeCommand(`git remote set-url origin "${remoteUrl}"`, clonePath)
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
}
