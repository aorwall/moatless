import crypto from 'node:crypto'
import path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { WorkspaceRuntimeStorage } from '../storage/workspace-runtime.storage'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import {
  createWorkspaceSnapshot,
  type WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'
import { WorkspaceMaterializer } from './workspace-materializer.service'

/**
 * WorkspaceCoordinator - Main orchestrator for workspace operations
 *
 * Manages workspace lifecycle through runtime snapshots:
 * - Snapshot-driven provisioning
 * - State persistence across restarts
 * - Coordination of preview/agent operations
 */
@injectable()
export class WorkspaceCoordinator {
  constructor(
    @inject(WorkspaceRuntimeStorage) private readonly runtimeStore: WorkspaceRuntimeStorage,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceMaterializer) private readonly materializer: WorkspaceMaterializer,
  ) {}

  /**
   * Ensure workspace is provisioned and ready
   * Returns handle to workspace or provisions a new one
   */
  async ensureWorkspace(taskId: string, repositoryId?: string): Promise<WorkspaceHandle> {
    logger.info('Ensuring workspace', { taskId, repositoryId })

    // 1. Try to load existing snapshot
    let snapshot = await this.runtimeStore.load(taskId)

    //  2. If snapshot exists and is valid, return existing workspace
    if (snapshot && (await this.isSnapshotValid(snapshot))) {
      logger.debug('Using existing workspace from snapshot', { taskId })
      return this.createHandle(snapshot)
    }

    // 3. Provision new workspace
    logger.info('Provisioning new workspace', { taskId, repositoryId })

    const repoId = repositoryId ?? snapshot?.repositoryId
    if (!repoId) {
      throw new Error(`Repository ID required to provision workspace for task ${taskId}`)
    }

    const repoConfig = await this.repoService.getRepositoryById(repoId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repoId}`)
    }

    const plan = this.createProvisionPlan(taskId, repoConfig)
    const handle = await this.materializer.applyPlan(taskId, plan)

    // 4. Create and save snapshot
    snapshot = createWorkspaceSnapshot(taskId, repoId, handle.workspacePath)
    snapshot.executionTarget.lastValidatedAt = new Date().toISOString()
    await this.runtimeStore.save(snapshot)

    logger.info('Workspace provisioned and snapshot saved', {
      taskId,
      workspacePath: handle.workspacePath,
    })

    return handle
  }

  /**
   * Get workspace runtime snapshot
   */
  async getSnapshot(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    return await this.runtimeStore.load(taskId)
  }

  /**
   * Update workspace snapshot
   */
  async updateSnapshot(
    taskId: string,
    updater: (snapshot: WorkspaceRuntimeSnapshot) => WorkspaceRuntimeSnapshot,
  ): Promise<boolean> {
    return await this.runtimeStore.update(taskId, updater)
  }

  /**
   * Delete workspace snapshot
   */
  async deleteSnapshot(taskId: string): Promise<void> {
    await this.runtimeStore.delete(taskId)
  }

  /**
   * Create workspace provision plan
   */
  private createProvisionPlan(
    taskId: string,
    repoConfig: RepositoryConfig,
  ): WorkspaceProvisionPlan {
    // Determine provision mode based on repository configuration
    const mode = this.determineProvisionMode(repoConfig)
    const branch = `workspace-${taskId}`

    const plan: WorkspaceProvisionPlan = {
      mode,
      branch,
      setupMetadataHash: this.computeSetupHash(repoConfig),
    }

    switch (mode) {
      case 'direct':
        // Use repository path directly
        plan.reusePath = repoConfig.path
        break

      case 'worktree':
        // Create worktree in workspace directory
        plan.reusePath = repoConfig.path
        plan.localTargetPath = this.getWorktreePath(repoConfig, taskId)
        break

      case 'clone':
        // Clone to new directory
        plan.reusePath = repoConfig.path
        plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        break
    }

    logger.debug('Created provision plan', { taskId, mode, branch })

    return plan
  }

  /**
   * Determine provisioning mode based on repository config
   */
  private determineProvisionMode(repoConfig: RepositoryConfig): 'direct' | 'worktree' | 'clone' {
    // If cloneOnNewTask is enabled, use clone mode
    if (repoConfig.cloneOnNewTask) {
      return 'clone'
    }

    // If useGitWorktrees is enabled, use worktree mode
    if (repoConfig.useGitWorktrees) {
      return 'worktree'
    }

    // Otherwise use direct mode (use repository path)
    return 'direct'
  }

  /**
   * Get worktree path for a task
   */
  private getWorktreePath(repoConfig: RepositoryConfig, taskId: string): string {
    const dataDir = getDataDir()
    const repoName = path.basename(repoConfig.path)
    const worktreeDir = path.join(dataDir, 'workspaces', 'worktrees')
    return path.join(worktreeDir, `${repoName}-workspace-${taskId}`)
  }

  /**
   * Get clone path for a task
   */
  private getClonePath(repoConfig: RepositoryConfig, taskId: string): string {
    const dataDir = getDataDir()
    const repoName = path.basename(repoConfig.path)
    const cloneDir = path.join(dataDir, 'workspaces', 'clones')
    return path.join(cloneDir, `${repoName}-clone-${taskId}`)
  }

  /**
   * Compute setup metadata hash for a repository config
   * Used to detect when configuration changes
   */
  private computeSetupHash(repoConfig: RepositoryConfig): string {
    const metadata = JSON.stringify({
      path: repoConfig.path,
      codeExecution: repoConfig.codeExecution,
      useGitWorktrees: repoConfig.useGitWorktrees,
      cloneOnNewTask: repoConfig.cloneOnNewTask,
    })
    return crypto.createHash('sha256').update(metadata).digest('hex').slice(0, 16)
  }

  /**
   * Check if a snapshot is still valid
   */
  private async isSnapshotValid(snapshot: WorkspaceRuntimeSnapshot): Promise<boolean> {
    // Check if workspace path still exists
    const workspacePath = snapshot.executionTarget.workspacePath
    const isValid = await this.materializer.validateWorkspace(workspacePath)

    if (!isValid) {
      logger.warn('Workspace snapshot invalid - path no longer exists', {
        taskId: snapshot.taskId,
        workspacePath,
      })
    }

    return isValid
  }

  /**
   * Create workspace handle from snapshot
   */
  private createHandle(snapshot: WorkspaceRuntimeSnapshot): WorkspaceHandle {
    return {
      taskId: snapshot.taskId,
      workspacePath: snapshot.executionTarget.workspacePath,
      mode: 'local',
    }
  }
}
