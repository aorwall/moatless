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
 * Error thrown when workspace path doesn't exist
 * This is a recoverable error - workspace can still be created with limited capabilities
 */
export class WorkspacePathMissingError extends Error {
  constructor(
    message: string,
    public readonly path: string,
  ) {
    super(message)
    this.name = 'WorkspacePathMissingError'
  }
}

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
    logger.debug({ taskId, mode: plan.mode }, 'Applying workspace provision plan')

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

    logger.debug({ taskId, workspacePath, mode: plan.mode }, 'Workspace materialized')

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
      throw new WorkspacePathMissingError(
        `Repository path does not exist: ${plan.reusePath}`,
        plan.reusePath,
      )
    }

    // Create and checkout the specified branch (for targetBranch support)
    if (plan.branch) {
      const currentBranch = await this.getCurrentBranch(plan.reusePath)
      if (currentBranch !== plan.branch) {
        const branchExistsFlag = await branchExists(plan.reusePath, plan.branch)
        if (!branchExistsFlag) {
          // Create the target branch from current HEAD
          await createBranch(plan.reusePath, plan.branch)
          logger.debug(
            { repoPath: plan.reusePath, branch: plan.branch },
            'Created target branch for task',
          )
        }
        await checkoutBranch(plan.reusePath, plan.branch)
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
  private async applyCloneMode(_taskId: string, plan: WorkspaceProvisionPlan): Promise<string> {
    if (!plan.localTargetPath) {
      throw new Error('Clone mode requires localTargetPath')
    }
    if (!plan.remoteUrl) {
      throw new Error('Clone mode requires remoteUrl')
    }

    // Ensure target directory exists
    const cloneDir = path.dirname(plan.localTargetPath)
    await mkdir(cloneDir, { recursive: true })

    // Clone repository (without branch option - we'll create it after)
    const workspacePath = await this.cloneRemoteRepository(plan, {})

    // If targetBranch is specified, create and checkout the branch after cloning
    if (plan.branch) {
      const branchExistsFlag = await branchExists(workspacePath, plan.branch)
      if (!branchExistsFlag) {
        // Create the target branch from default branch HEAD
        await createBranch(workspacePath, plan.branch)
        logger.debug(
          { workspacePath, branch: plan.branch },
          'Created target branch for task after clone',
        )
      }
      await checkoutBranch(workspacePath, plan.branch)
    }

    return workspacePath
  }

  /**
   * Clone from remote repository URL with authentication
   */
  private async cloneRemoteRepository(
    plan: WorkspaceProvisionPlan,
    cloneOptions: { branch?: string },
  ): Promise<string> {
    if (!plan.provider) {
      throw new Error(
        'Provider is required for remote repository cloning. Only GitHub and GitLab are supported.',
      )
    }
    if (!(plan.remoteUrl && plan.localTargetPath)) {
      throw new Error('remoteUrl and localTargetPath are required')
    }

    const provider = plan.provider
    const token = await this.gitProviderTokenService.getTokenForProvider(provider)
    const cloneUrl = this.gitProviderTokenService.buildAuthenticatedCloneUrl(
      plan.remoteUrl,
      token,
      provider,
    )

    logger.info(
      {
        remoteUrl: plan.remoteUrl,
        targetPath: plan.localTargetPath,
        provider,
        hasAuth: Boolean(token),
        branch: plan.branch,
      },
      'Cloning remote repository',
    )

    try {
      await cloneRepository(cloneUrl, plan.localTargetPath, cloneOptions)
    } catch (error) {
      const errorMessage = this.buildCloneErrorMessage(error, plan.remoteUrl, provider, token)
      throw new Error(errorMessage)
    }

    // Remove PAT from git remote config for security
    if (token) {
      await this.removePATFromRemote(plan.remoteUrl, plan.localTargetPath)
    }

    return plan.localTargetPath
  }

  /**
   * Build user-friendly error message for clone failures
   */
  private buildCloneErrorMessage(
    error: unknown,
    remoteUrl: string,
    provider: string,
    token: string | null,
  ): string {
    // Sanitize error message to remove any tokens
    const errorMessage = error instanceof Error ? error.message : String(error)
    const sanitizedError = token ? errorMessage.replace(token, '***') : errorMessage

    logger.error(
      { err: error as Error, remoteUrl, provider, hasAuth: Boolean(token) },
      'Failed to clone remote repository',
    )

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

    return userMessage
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
      logger.debug({ workspacePath }, 'Removed PAT from git remote config')
    } catch (error) {
      // Log but don't fail - this is a security cleanup step
      logger.warn(
        {
          workspacePath,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to remove PAT from git remote',
      )
    }
  }

  /**
   * Clean up workspace artifacts based on provision plan
   * This is the symmetric operation to applyPlan()
   * Currently only handles clone mode
   *
   * @param taskId - Task ID
   * @param plan - Original provision plan used to create workspace
   */
  async cleanupWorkspace(taskId: string, plan: WorkspaceProvisionPlan): Promise<void> {
    logger.debug({ taskId, mode: plan.mode }, 'Cleaning up workspace artifacts')

    try {
      switch (plan.mode) {
        case 'direct':
          // No cleanup needed - workspace uses repository path directly
          logger.debug({ taskId }, 'Direct mode: No workspace artifacts to clean up')
          break

        case 'worktree':
          // Worktree cleanup not implemented yet (out of scope for this implementation)
          logger.debug({ taskId }, 'Worktree mode: Cleanup not implemented (out of scope)')
          break

        case 'clone':
          await this.cleanupCloneMode(taskId, plan)
          break

        default:
          logger.warn({ taskId, mode: plan.mode }, 'Unknown provision mode for cleanup')
      }

      logger.debug({ taskId, mode: plan.mode }, 'Workspace artifacts cleaned up successfully')
    } catch (error) {
      logger.error(
        { err: error as Error, taskId, mode: plan.mode },
        'Failed to clean up workspace artifacts',
      )
      // Don't throw - cleanup should be best-effort
    }
  }

  /**
   * Clean up clone mode artifacts
   * Removes the cloned directory
   */
  private async cleanupCloneMode(taskId: string, plan: WorkspaceProvisionPlan): Promise<void> {
    if (!plan.localTargetPath) {
      logger.warn({ taskId }, 'Clone cleanup: Missing localTargetPath in plan')
      return
    }

    try {
      const { removeDirectory } = await import('../../../utils/filesystem.js')
      await removeDirectory(plan.localTargetPath)
      logger.info(
        { taskId, clonePath: plan.localTargetPath },
        'Clone directory removed successfully',
      )
    } catch (error) {
      logger.error(
        { err: error as Error, taskId, clonePath: plan.localTargetPath },
        'Failed to remove clone directory',
      )
    }
  }
}
