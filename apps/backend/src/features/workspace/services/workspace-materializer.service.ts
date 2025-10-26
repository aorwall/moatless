import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { injectable } from 'inversify'
import { pathExists } from '../../../utils/filesystem'
import { logger } from '../../../utils/logger'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
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
  /**
   * Apply a provisioning plan to materialize a workspace
   */
  async applyPlan(taskId: string, plan: WorkspaceProvisionPlan): Promise<WorkspaceHandle> {
    logger.info('Applying workspace provision plan', { taskId, mode: plan.mode })

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

    logger.info('Workspace materialized', { taskId, workspacePath, mode: plan.mode })

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
  private async applyCloneMode(_taskId: string, plan: WorkspaceProvisionPlan): Promise<string> {
    if (!plan.localTargetPath) {
      throw new Error('Clone mode requires localTargetPath')
    }

    // If reusePath is provided, clone from local path
    if (plan.reusePath) {
      const cloneDir = path.dirname(plan.localTargetPath)
      await mkdir(cloneDir, { recursive: true })

      // Only pass branch option if branch is defined
      const cloneOptions = plan.branch ? { branch: plan.branch } : {}
      await cloneRepository(plan.reusePath, plan.localTargetPath, cloneOptions)

      return plan.localTargetPath
    }

    throw new Error('Clone mode requires either reusePath or remoteClone configuration')
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
}
