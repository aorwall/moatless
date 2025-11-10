import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { inject, injectable } from 'inversify'
import { pathExists } from '../../../utils/filesystem'
import { logger } from '../../../utils/logger'
import { GitProviderTokenService } from '../../auth/services/git-provider-token.service'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { BaseWorkspaceHandle } from '../types/workspace-handle.types'
import {
  branchExists,
  checkoutBranch,
  cloneRepository,
  createBranch,
  createGitWorktree,
  isGitRepository,
} from '../utils/git.utils'

/**
 * WorkspaceMaterializer - Executes workspace provisioning plans
 * Handles local filesystem operations for workspace setup
 */
@injectable()
export class WorkspaceMaterializer {
  constructor(
    @inject(GitProviderTokenService)
    private readonly gitProviderTokenService: GitProviderTokenService,
  ) {}
  /**
   * Apply a provisioning plan to materialize a workspace
   * Returns base handle without runtime state (enrichment happens in RuntimeManager)
   */
  async applyPlan(taskId: string, plan: WorkspaceProvisionPlan): Promise<BaseWorkspaceHandle> {
    logger.debug('Applying workspace provision plan', { taskId, mode: plan.mode })

    let workspacePath: string

    switch (plan.mode) {
      case 'direct':
        workspacePath = await this.applyDirectMode(plan)
        break

      case 'worktree':
        workspacePath = await this.applyWorktreeMode(taskId, plan)
        break

      case 'clone':
        workspacePath = await this.applyCloneMode(taskId, plan)
        break

      default:
        throw new Error(`Unsupported provision mode: ${plan.mode}`)
    }

    logger.debug('Workspace materialized', { taskId, workspacePath, mode: plan.mode })

    return {
      taskId,
      workspacePath,
      mode: 'local',
    }
  }

  /**
   * Direct mode: Use repository path directly
   */
  private async applyDirectMode(plan: WorkspaceProvisionPlan): Promise<string> {
    if (!plan.reusePath) {
      throw new Error('Direct mode requires reusePath')
    }

    const exists = await pathExists(plan.reusePath)
    if (!exists) {
      throw new Error(`Repository path does not exist: ${plan.reusePath}`)
    }

    // Optionally checkout the specified branch
    if (plan.branch) {
      const currentBranch = await this.getCurrentBranch(plan.reusePath)
      if (currentBranch !== plan.branch) {
        const branchExistsFlag = await branchExists(plan.reusePath, plan.branch)
        if (branchExistsFlag) {
          await checkoutBranch(plan.reusePath, plan.branch)
        }
      }
    }

    return plan.reusePath
  }

  /**
   * Worktree mode: Create a git worktree
   */
  private async applyWorktreeMode(_taskId: string, plan: WorkspaceProvisionPlan): Promise<string> {
    if (!plan.reusePath) {
      throw new Error('Worktree mode requires reusePath (main repo)')
    }

    if (!plan.localTargetPath) {
      throw new Error('Worktree mode requires localTargetPath')
    }

    // Verify main repo is a git repository
    const isRepo = await isGitRepository(plan.reusePath)
    if (!isRepo) {
      throw new Error(`Path is not a git repository: ${plan.reusePath}`)
    }

    // Create worktree directory if needed
    const worktreeDir = path.dirname(plan.localTargetPath)
    await mkdir(worktreeDir, { recursive: true })

    // Branch is required for worktree mode
    if (!plan.branch) {
      throw new Error('Worktree mode requires a branch name')
    }

    // Create branch if it doesn't exist
    const branchExistsFlag = await branchExists(plan.reusePath, plan.branch)
    if (!branchExistsFlag) {
      await createBranch(plan.reusePath, plan.branch)
    }

    // Create worktree
    await createGitWorktree(plan.reusePath, plan.localTargetPath, plan.branch)

    return plan.localTargetPath
  }

  /**
   * Clone mode: Clone repository to new location
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex error handling for different clone scenarios
  private async applyCloneMode(_taskId: string, plan: WorkspaceProvisionPlan): Promise<string> {
    if (!plan.localTargetPath) {
      throw new Error('Clone mode requires localTargetPath')
    }

    // Ensure target directory exists
    const cloneDir = path.dirname(plan.localTargetPath)
    await mkdir(cloneDir, { recursive: true })

    // Clone options (include branch if specified)
    const cloneOptions = plan.branch ? { branch: plan.branch } : {}

    // If reusePath is provided, clone from local path (local/template repos)
    if (plan.reusePath) {
      await cloneRepository(plan.reusePath, plan.localTargetPath, cloneOptions)
      return plan.localTargetPath
    }

    // If remoteUrl is provided, clone from remote URL (remote repos)
    if (plan.remoteUrl) {
      // Use provider from plan (already determined at repository creation)
      if (!plan.provider) {
        throw new Error(
          'Provider is required for remote repository cloning. Only GitHub and GitLab are supported.',
        )
      }
      const provider = plan.provider
      const token = await this.gitProviderTokenService.getTokenForProvider(provider)

      // Build authenticated clone URL if token is available
      const cloneUrl = this.gitProviderTokenService.buildAuthenticatedCloneUrl(
        plan.remoteUrl,
        token,
        provider,
      )

      logger.info('Cloning remote repository', {
        remoteUrl: plan.remoteUrl,
        targetPath: plan.localTargetPath,
        provider,
        hasAuth: Boolean(token),
        branch: plan.branch,
      })

      try {
        await cloneRepository(cloneUrl, plan.localTargetPath, cloneOptions)
      } catch (error) {
        // Sanitize error message to remove any tokens
        const errorMessage = error instanceof Error ? error.message : String(error)
        const sanitizedError = token ? errorMessage.replace(token, '***') : errorMessage

        logger.error('Failed to clone remote repository', error as Error, {
          remoteUrl: plan.remoteUrl,
          targetPath: plan.localTargetPath,
          provider,
          hasAuth: Boolean(token),
        })

        // Provide helpful error message
        let userMessage = `Failed to clone repository: ${sanitizedError}`
        if (!token) {
          if (provider === 'github') {
            userMessage +=
              '\n\nConsider setting up a GitHub service connection for authentication (via GitHub OAuth or Personal Access Token).'
          } else if (provider === 'gitlab') {
            userMessage +=
              '\n\nConsider setting up a GitLab Personal Access Token in Settings > GitLab.'
          } else {
            userMessage += '\n\nConsider setting up authentication for your Git provider.'
          }
        }

        throw new Error(userMessage)
      }

      // Remove PAT from git remote config for security
      if (token) {
        await this.removePATFromRemote(plan.remoteUrl, plan.localTargetPath)
      }

      return plan.localTargetPath
    }

    throw new Error('Clone mode requires either reusePath or remoteUrl')
  }

  /**
   * Get current branch of a repository
   */
  private async getCurrentBranch(repoPath: string): Promise<string> {
    const { exec } = await import('node:child_process')
    const { promisify } = await import('node:util')
    const execAsync = promisify(exec)

    const { stdout } = await execAsync('git rev-parse --abbrev-ref HEAD', { cwd: repoPath })
    return stdout.trim()
  }

  /**
   * Validate that a workspace exists and is accessible
   */
  async validateWorkspace(workspacePath: string): Promise<boolean> {
    return await pathExists(workspacePath)
  }

  /**
   * Remove PAT from git remote config (security best practice)
   */
  private async removePATFromRemote(remoteUrl: string, workspacePath: string): Promise<void> {
    try {
      const { exec } = await import('node:child_process')
      const { promisify } = await import('node:util')
      const execAsync = promisify(exec)

      await execAsync(`git remote set-url origin "${remoteUrl}"`, { cwd: workspacePath })
      logger.debug('Removed PAT from git remote config', { workspacePath })
    } catch (error) {
      // Log but don't fail - this is a security cleanup step
      logger.warn('Failed to remove PAT from git remote', {
        workspacePath,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
}
