import crypto from 'node:crypto'
import path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeRepository } from '../db/workspace-runtime.repository'
import { LocalEnvironmentDriver } from '../drivers/local-environment-driver'
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
 * This is the ONLY service that directly accesses WorkspaceRuntimeRepository.
 *
 * Responsibilities:
 * - Workspace provisioning and validation
 * - Runtime snapshot CRUD operations
 * - State updates for preview and agent
 * - Returns enriched WorkspaceHandle with runtime state
 *
 * Design principle: Consumers should NOT access RuntimeRepository directly.
 * All state access/updates flow through this manager.
 */
@injectable()
export class WorkspaceRuntimeManager {
  constructor(
    @inject(WorkspaceRuntimeRepository) private readonly runtimeStore: WorkspaceRuntimeRepository,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceMaterializer) private readonly materializer: WorkspaceMaterializer,
    @inject(LocalEnvironmentDriver) private readonly driver: LocalEnvironmentDriver,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
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
   *
   * Auto-recovers missing workspace snapshots by re-initializing workspace
   * if task has a valid repositoryId.
   */
  async getWorkspacePath(taskId: string): Promise<string> {
    const snapshot = await this.runtimeStore.load(taskId)

    // Auto-recovery: if snapshot missing, try to re-initialize workspace
    if (!snapshot) {
      logger.warn(
        'WorkspaceRuntimeManager: No workspace snapshot found, attempting auto-recovery',
        {
          taskId,
        },
      )

      // Get task to find repositoryId
      const repositoryId = await this.getTaskRepositoryId(taskId)
      if (!repositoryId) {
        throw new Error(
          `Cannot auto-recover workspace for task ${taskId}: task has no repositoryId configured. ` +
            'Please ensure the task is properly initialized with a repository.',
        )
      }

      // Auto-provision workspace
      logger.info('WorkspaceRuntimeManager: Auto-recovering workspace', {
        taskId,
        repositoryId,
      })

      const handle = await this.ensureWorkspace(taskId, repositoryId)

      logger.info('WorkspaceRuntimeManager: Workspace auto-recovery successful', {
        taskId,
        workspacePath: handle.workspacePath,
      })

      return handle.workspacePath
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

  /**
   * Update workspace paths for all snapshots of a repository when the repository directory is renamed.
   * Only updates direct-mode workspaces that use the repository path directly.
   * Worktree/clone mode workspaces have independent paths and don't need updating.
   *
   * @param repositoryId - Repository ID
   * @param oldPath - Old repository path
   * @param newPath - New repository path
   */
  async updateWorkspacePathsForRepository(
    repositoryId: string,
    oldPath: string,
    newPath: string,
  ): Promise<void> {
    logger.info('Updating workspace paths for repository rename', {
      repositoryId,
      oldPath,
      newPath,
    })

    // Get all snapshots
    const allSnapshots = await this.runtimeStore.list()

    // Filter for snapshots that:
    // 1. Belong to this repository
    // 2. Use direct mode (workspacePath equals the old repository path)
    const affectedSnapshots = allSnapshots.filter(
      (snapshot) =>
        snapshot.repositoryId === repositoryId &&
        snapshot.executionTarget.workspacePath === oldPath,
    )

    logger.info('Found affected workspace snapshots', {
      repositoryId,
      count: affectedSnapshots.length,
      taskIds: affectedSnapshots.map((s) => s.taskId),
    })

    // Update each affected snapshot
    for (const snapshot of affectedSnapshots) {
      await this.runtimeStore.update(snapshot.taskId, (s) => ({
        ...s,
        executionTarget: {
          ...s.executionTarget,
          workspacePath: newPath,
        },
      }))

      logger.debug('Updated workspace path for snapshot', {
        taskId: snapshot.taskId,
        oldPath,
        newPath,
      })
    }

    logger.info('Workspace paths updated successfully', {
      repositoryId,
      updatedCount: affectedSnapshots.length,
    })
  }

  // ========================================
  // Private Helper Methods
  // ========================================

  /**
   * Get repositoryId from a task
   * Returns null if task not found or has no repositoryId
   */
  private async getTaskRepositoryId(taskId: string): Promise<string | null> {
    const task = await this.taskRepository.get(taskId)
    return task?.repositoryId || null
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
        // Use repository path directly (local/template only)
        if (repoConfig.kind === 'local' || repoConfig.kind === 'template') {
          plan.reusePath = repoConfig.path
        } else {
          throw new Error('Direct mode only supported for local/template repositories')
        }
        break

      case 'worktree':
        // Create worktree in workspace directory (local/template only)
        if (repoConfig.kind === 'local' || repoConfig.kind === 'template') {
          plan.reusePath = repoConfig.path
          plan.localTargetPath = this.getWorktreePath(repoConfig, taskId)
        } else {
          throw new Error('Worktree mode only supported for local/template repositories')
        }
        break

      case 'clone':
        // Clone to new directory
        if (repoConfig.kind === 'remote') {
          // For remote repos, clone from URL (no reusePath needed)
          plan.remoteUrl = repoConfig.remote.url
          plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        } else if (repoConfig.kind === 'local' || repoConfig.kind === 'template') {
          // For local/template repos, clone from local path
          plan.reusePath = repoConfig.path
          plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        }
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
    switch (repoConfig.kind) {
      case 'remote':
        // Remote repositories always clone per task
        return 'clone'

      case 'local':
      case 'template':
        // Use worktree mode if enabled, otherwise direct
        return repoConfig.useGitWorktrees ? 'worktree' : 'direct'

      default: {
        const _exhaustiveCheck: never = repoConfig
        throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
      }
    }
  }

  /**
   * Get worktree path for a task
   */
  private getWorktreePath(repoConfig: RepositoryConfig, taskId: string): string {
    const dataDir = getDataDir()
    let repoName: string

    if (repoConfig.kind === 'local' || repoConfig.kind === 'template') {
      repoName = path.basename(repoConfig.path)
    } else {
      throw new Error('Worktree path only supported for local/template repositories')
    }

    const worktreeDir = path.join(dataDir, 'workspaces', 'worktrees')
    return path.join(worktreeDir, `${repoName}-workspace-${taskId}`)
  }

  /**
   * Get clone path for a task
   */
  private getClonePath(repoConfig: RepositoryConfig, taskId: string): string {
    const dataDir = getDataDir()
    let repoName: string

    if (repoConfig.kind === 'local' || repoConfig.kind === 'template') {
      repoName = path.basename(repoConfig.path)
    } else if (repoConfig.kind === 'remote') {
      // For remote repos, use repository name from config
      repoName = repoConfig.name
    } else {
      const _exhaustiveCheck: never = repoConfig
      throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
    }

    const cloneDir = path.join(dataDir, 'workspaces', 'clones')
    return path.join(cloneDir, `${repoName}-clone-${taskId}`)
  }

  /**
   * Compute setup metadata hash for a repository config
   * Used to detect when configuration changes
   */
  private computeSetupHash(repoConfig: RepositoryConfig): string {
    let metadata: Record<string, unknown>

    switch (repoConfig.kind) {
      case 'local':
      case 'template':
        metadata = {
          kind: repoConfig.kind,
          path: repoConfig.path,
          codeExecution: repoConfig.codeExecution,
          useGitWorktrees: repoConfig.useGitWorktrees,
        }
        break

      case 'remote':
        metadata = {
          kind: repoConfig.kind,
          remoteUrl: repoConfig.remote.url,
          codeExecution: repoConfig.codeExecution,
        }
        break

      default: {
        const _exhaustiveCheck: never = repoConfig
        throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
      }
    }

    return crypto.createHash('sha256').update(JSON.stringify(metadata)).digest('hex').slice(0, 16)
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
