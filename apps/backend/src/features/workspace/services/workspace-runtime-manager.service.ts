import crypto from 'node:crypto'
import path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { LocalEnvironmentDriver } from '../drivers/local-environment-driver'
import { WorkspaceRuntimeStorage } from '../storage/workspace-runtime.storage'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import {
  type AgentState,
  createWorkspaceSnapshot,
  type PreviewState,
  type WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'
import { WorkspaceMaterializer } from './workspace-materializer.service'

/**
 * WorkspaceRuntimeManager - Low-level runtime state manager
 *
 * Owns all workspace provisioning and runtime state management.
 * This is the ONLY service that directly accesses WorkspaceRuntimeStorage.
 *
 * Responsibilities:
 * - Workspace provisioning and validation
 * - Runtime snapshot CRUD operations
 * - State updates for preview and agent
 * - Returns enriched WorkspaceHandle with runtime state
 *
 * Design principle: Consumers should NOT access RuntimeStorage directly.
 * All state access/updates flow through this manager.
 */
@injectable()
export class WorkspaceRuntimeManager {
  constructor(
    @inject(WorkspaceRuntimeStorage) private readonly runtimeStore: WorkspaceRuntimeStorage,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceMaterializer) private readonly materializer: WorkspaceMaterializer,
    @inject(LocalEnvironmentDriver) private readonly driver: LocalEnvironmentDriver,
  ) {}

  /**
   * Ensure workspace is provisioned and ready
   * Returns enriched handle with current runtime state
   */
  async ensureWorkspace(taskId: string, repositoryId?: string): Promise<WorkspaceHandle> {
    logger.info('WorkspaceRuntimeManager: Ensuring workspace', { taskId, repositoryId })

    // 1. Try to load existing snapshot
    let snapshot = await this.runtimeStore.load(taskId)

    //  2. If snapshot exists and is valid, return existing workspace
    if (snapshot && (await this.isSnapshotValid(snapshot))) {
      logger.debug('WorkspaceRuntimeManager: Using existing workspace from snapshot', { taskId })
      return this.createEnrichedHandle(snapshot)
    }

    // 3. Provision new workspace
    logger.info('WorkspaceRuntimeManager: Provisioning new workspace', { taskId, repositoryId })

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
    snapshot.executionTarget.lastValidatedAt = new Date()
    await this.runtimeStore.save(snapshot)

    logger.info('WorkspaceRuntimeManager: Workspace provisioned and snapshot saved', {
      taskId,
      workspacePath: handle.workspacePath,
    })

    return this.createEnrichedHandle(snapshot)
  }

  /**
   * Get workspace runtime snapshot
   */
  async getSnapshot(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    return await this.runtimeStore.load(taskId)
  }

  /**
   * Get workspace path from runtime snapshot
   * Used by FileGateway for taskId-only file operations
   */
  async getWorkspacePath(taskId: string): Promise<string> {
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      throw new Error(`No workspace snapshot found for task ${taskId}`)
    }

    const workspacePath = snapshot.executionTarget.workspacePath
    if (!workspacePath) {
      throw new Error(`Workspace not provisioned for task ${taskId}`)
    }

    return workspacePath
  }

  /**
   * Update preview server state in runtime snapshot
   */
  async updatePreviewState(taskId: string, updates: Partial<PreviewState>): Promise<void> {
    await this.runtimeStore.update(taskId, (s) => ({
      ...s,
      preview: {
        ...s.preview,
        ...updates,
      },
    }))
  }

  /**
   * Update agent execution state in runtime snapshot
   */
  async updateAgentState(taskId: string, updates: Partial<AgentState>): Promise<void> {
    await this.runtimeStore.update(taskId, (s) => ({
      ...s,
      agent: {
        state: 'idle' as const, // Default state
        ...s.agent, // Existing state
        ...updates, // Updates override
      } as AgentState, // Type assertion
    }))
  }

  /**
   * Stop running agent for a task
   * Delegates to the driver layer
   */
  async stopAgent(taskId: string): Promise<void> {
    await this.driver.stopAgent(taskId)
  }

  /**
   * Delete workspace snapshot
   */
  async deleteSnapshot(taskId: string): Promise<void> {
    await this.runtimeStore.delete(taskId)
  }

  // ========================================
  // Private Helper Methods
  // ========================================

  /**
   * Create workspace provision plan
   */
  private createProvisionPlan(
    taskId: string,
    repoConfig: RepositoryConfig,
  ): WorkspaceProvisionPlan {
    // Determine provision mode based on repository configuration
    const mode = this.determineProvisionMode(repoConfig)

    // Only create custom branch for worktree mode
    // Clone mode uses default branch, direct mode may checkout existing branch
    const branch = mode === 'worktree' ? `workspace-${taskId}` : undefined

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
        // Clone to new directory (will use default branch)
        plan.reusePath = repoConfig.path
        plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        break
    }

    logger.debug('WorkspaceRuntimeManager: Created provision plan', {
      taskId,
      mode,
      branch: branch || 'default',
    })

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
      logger.warn('WorkspaceRuntimeManager: Workspace snapshot invalid - path no longer exists', {
        taskId: snapshot.taskId,
        workspacePath,
      })
    }

    return isValid
  }

  /**
   * Create enriched workspace handle from snapshot
   * Includes all runtime state needed by consumers
   */
  private createEnrichedHandle(snapshot: WorkspaceRuntimeSnapshot): WorkspaceHandle {
    return {
      taskId: snapshot.taskId,
      workspacePath: snapshot.executionTarget.workspacePath,
      repositoryId: snapshot.repositoryId,
      mode: 'local',
      preview: {
        status: snapshot.preview.status,
        lastStartedAt: snapshot.preview.lastStartedAt,
        lastStoppedAt: snapshot.preview.lastStoppedAt,
        lastError: snapshot.preview.lastError,
        needsSetup: !snapshot.preview.lastStartedAt, // Derived: true if never started
      },
      agent: {
        state: snapshot.agent?.state ?? ('idle' as const),
        sessionId: snapshot.agent?.sessionId,
        agentType: snapshot.agent?.agentType,
        startedAt: snapshot.agent?.startedAt,
      },
    }
  }
}
