import { randomUUID } from 'node:crypto'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { activeWorkspaces } from '../../metrics'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeRepository } from '../db/workspace-runtime.repository'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import type { WorkspaceProvisioner } from '../types/workspace-provisioner.types'
import {
  type AgentState,
  createWorkspaceSnapshotFromTarget,
  type ExecutionTarget,
  getWorkspacePathFromTarget,
  isLocalExecutionTarget,
  type PreviewState,
  type WorkspaceLock,
  type WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'
import { WorkspaceProvisionPlanService } from './workspace-provision-plan.service'

const SETUP_LOCK_TTL_MS = 10 * 60 * 1000
const SETUP_WAIT_TIMEOUT_MS = 10 * 60 * 1000
const SETUP_WAIT_INTERVAL_MS = 2000

/**
 * WorkspaceRuntimeManager - Low-level runtime state manager
 *
 * Owns all workspace provisioning, runtime state management, and driver lifecycle.
 * This is the ONLY service that directly accesses WorkspaceRuntimeRepository.
 *
 * Responsibilities:
 * - Workspace provisioning and validation
 * - Runtime snapshot CRUD operations
 * - State updates for preview and agent
 * - Driver creation (per-task)
 * - Returns enriched WorkspaceHandle with runtime state and driver
 *
 * Design principle: Consumers should NOT access RuntimeRepository directly.
 * All state access/updates flow through this manager.
 * Environment drivers are always obtained through WorkspaceHandle.driver.
 */
@injectable()
export class WorkspaceRuntimeManager {
  private readonly instanceId = randomUUID()

  constructor(
    @inject(WorkspaceRuntimeRepository) private readonly runtimeStore: WorkspaceRuntimeRepository,
    @inject(WorkspaceProvisionPlanService)
    private readonly planService: WorkspaceProvisionPlanService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject('WorkspaceProvisioner')
    private readonly provisioner: WorkspaceProvisioner,
    @inject(EventService) private readonly eventService: EventService,
  ) {}

  /**
   * Ensure workspace is provisioned and ready
   * Returns enriched handle with current runtime state
   * Handles gracefully when workspace path doesn't exist (returns unavailable workspace)
   */
  async ensureWorkspace(taskId: string, repositoryId?: string): Promise<WorkspaceHandle> {
    logger.debug({ taskId, repositoryId }, 'WorkspaceRuntimeManager: Ensuring workspace')

    // 1. Try to load existing snapshot
    const snapshot = await this.runtimeStore.load(taskId)

    const existingHandle = await this.tryGetExistingHandle(snapshot)
    if (existingHandle) {
      return existingHandle
    }

    const repoId = repositoryId ?? snapshot?.repositoryId
    if (!repoId) {
      throw new Error(`Repository ID required to provision workspace for task ${taskId}`)
    }

    const { plan, initializingTarget } = await this.prepareProvisioning(taskId, repoId)
    const { lock, acquired } = await this.acquireSetupLock(
      taskId,
      repoId,
      snapshot,
      initializingTarget,
    )

    if (!acquired) {
      return await this.waitForProvisioning(taskId, repoId)
    }

    try {
      return await this.provisionAndPersist(taskId, repoId, plan, initializingTarget)
    } finally {
      if (lock) {
        await this.runtimeStore.releaseLock(taskId, lock.type, lock.owner)
      }
    }
  }

  /**
   * Get workspace runtime snapshot
   */
  async getSnapshot(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    return await this.runtimeStore.load(taskId)
  }

  /**
   * Get workspace handle without provisioning or validation.
   * Returns null if no snapshot exists.
   */
  async getExistingWorkspaceHandle(taskId: string): Promise<WorkspaceHandle | null> {
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      return null
    }

    return this.createEnrichedHandle(snapshot)
  }

  /**
   * Get snapshot with live environment validation
   * - For Kubernetes: checks if pod actually exists and updates phase
   * - For local: checks if workspace path exists
   *
   * Use this method when you need accurate real-time status (e.g., for status endpoints).
   * Use getSnapshot() when you just need the persisted state.
   */
  async getValidatedSnapshot(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      return null
    }

    const refreshedTarget = await this.provisioner.refreshExecutionTarget(
      taskId,
      snapshot.executionTarget,
    )

    return {
      ...snapshot,
      executionTarget: refreshedTarget,
    }
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
        {
          taskId,
        },
        'WorkspaceRuntimeManager: No workspace snapshot found, attempting auto-recovery',
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
      logger.info(
        {
          taskId,
          repositoryId,
        },
        'WorkspaceRuntimeManager: Auto-recovering workspace',
      )

      const handle = await this.ensureWorkspace(taskId, repositoryId)

      logger.info(
        {
          taskId,
          workspacePath: handle.workspacePath,
        },
        'WorkspaceRuntimeManager: Workspace auto-recovery successful',
      )

      return handle.workspacePath
    }

    const workspacePath = getWorkspacePathFromTarget(snapshot.executionTarget)
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
   * Check if an agent is actually running by querying the worker.
   * This validates that the database state matches reality.
   *
   * For Kubernetes targets: Queries the worker pod's /agent-status endpoint
   * For local targets: Returns the database state (no separate worker process)
   *
   * @param taskId - Task ID to check
   * @returns true if agent is confirmed running, false if idle/unreachable
   */
  async isAgentActuallyRunning(taskId: string): Promise<boolean> {
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      return false // No snapshot = no agent
    }

    // For local targets, trust the database state (no separate worker to query)
    // Local agents run in the same process, so if the state says running,
    // we trust it (cleanup should have reset it if the agent crashed)
    if (isLocalExecutionTarget(snapshot.executionTarget)) {
      return snapshot.agent?.state === 'running'
    }

    return await this.provisioner.isAgentRunning(taskId, snapshot.executionTarget)
  }

  /**
   * Delete workspace snapshot
   */
  async deleteSnapshot(taskId: string): Promise<void> {
    // Get snapshot to determine mode for metrics
    const snapshot = await this.runtimeStore.load(taskId)
    if (snapshot) {
      const mode = isLocalExecutionTarget(snapshot.executionTarget) ? 'local' : 'kubernetes'
      if (
        !isLocalExecutionTarget(snapshot.executionTarget) ||
        snapshot.executionTarget.isAvailable !== false
      ) {
        activeWorkspaces.labels({ mode }).dec()
      }
    }

    await this.runtimeStore.delete(taskId)
  }

  /**
   * Clear workspace error state and prepare for retry
   * Deletes failed Kubernetes pod if exists and clears the error snapshot
   */
  async clearWorkspaceError(taskId: string): Promise<void> {
    logger.info({ taskId }, 'WorkspaceRuntimeManager: Clearing workspace error state')

    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      logger.debug({ taskId }, 'No snapshot found to clear')
      return
    }

    try {
      await this.provisioner.clearError(taskId, snapshot.executionTarget)
    } catch (error) {
      logger.debug(
        { taskId, error: (error as Error).message },
        'Failed to clear workspace error via provisioner',
      )
    }

    // Delete the error snapshot to allow fresh provisioning
    await this.runtimeStore.delete(taskId)
    logger.info({ taskId }, 'WorkspaceRuntimeManager: Workspace error state cleared')
  }

  /**
   * Clean up workspace artifacts before deleting snapshot
   * Reconstructs provision plan from snapshot to determine cleanup actions
   *
   * This method:
   * 1. Loads workspace snapshot
   * 2. Reconstructs the provision plan used to create workspace
   * 3. Delegates cleanup to the provisioner (local materializer or k8s pod cleanup)
   *
   * @param taskId - Task ID
   */
  async cleanupWorkspaceArtifacts(taskId: string): Promise<void> {
    logger.debug({ taskId }, 'WorkspaceRuntimeManager: Cleaning up workspace artifacts')

    try {
      // 1. Load snapshot to get workspace configuration
      const snapshot = await this.runtimeStore.load(taskId)
      if (!snapshot) {
        logger.debug({ taskId }, 'No workspace snapshot found, skipping artifact cleanup')
        return
      }

      let plan: WorkspaceProvisionPlan | undefined
      if (isLocalExecutionTarget(snapshot.executionTarget)) {
        try {
          const repoConfig = await this.planService.getRepositoryConfig(snapshot.repositoryId)
          plan = await this.planService.createProvisionPlan(taskId, repoConfig)
        } catch (error) {
          logger.warn(
            { taskId, repositoryId: snapshot.repositoryId, error: String(error) },
            'Repository not found, cannot reconstruct provision plan for cleanup',
          )
          return
        }
      }

      await this.provisioner.cleanup(taskId, snapshot.executionTarget, plan)

      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Workspace artifacts cleaned up')
    } catch (error) {
      logger.error(
        { err: error as Error, taskId },
        'WorkspaceRuntimeManager: Failed to clean up workspace artifacts',
      )
      // Don't throw - cleanup should be best-effort
    }
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
    logger.info(
      {
        repositoryId,
        oldPath,
        newPath,
      },
      'Updating workspace paths for repository rename',
    )

    // Get all snapshots
    const allSnapshots = await this.runtimeStore.list()

    // Filter for snapshots that:
    // 1. Belong to this repository
    // 2. Use local execution mode with direct path (workspacePath equals the old repository path)
    const affectedSnapshots = allSnapshots.filter(
      (snapshot) =>
        snapshot.repositoryId === repositoryId &&
        isLocalExecutionTarget(snapshot.executionTarget) &&
        snapshot.executionTarget.workspacePath === oldPath,
    )

    logger.info(
      {
        repositoryId,
        count: affectedSnapshots.length,
        taskIds: affectedSnapshots.map((s) => s.taskId),
      },
      'Found affected workspace snapshots',
    )

    // Update each affected snapshot (only local execution targets)
    for (const snapshot of affectedSnapshots) {
      await this.runtimeStore.update(snapshot.taskId, (s) => {
        if (!isLocalExecutionTarget(s.executionTarget)) {
          return s // Skip non-local targets
        }
        return {
          ...s,
          executionTarget: {
            ...s.executionTarget,
            workspacePath: newPath,
          },
        }
      })

      logger.debug(
        {
          taskId: snapshot.taskId,
          oldPath,
          newPath,
        },
        'Updated workspace path for snapshot',
      )
    }

    logger.info(
      {
        repositoryId,
        updatedCount: affectedSnapshots.length,
      },
      'Workspace paths updated successfully',
    )
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

  private async tryGetExistingHandle(
    snapshot: WorkspaceRuntimeSnapshot | null,
  ): Promise<WorkspaceHandle | null> {
    if (!snapshot) {
      return null
    }

    if (await this.isSnapshotValid(snapshot)) {
      logger.debug({ taskId: snapshot.taskId }, 'WorkspaceRuntimeManager: Using existing workspace')
      return this.createEnrichedHandle(snapshot)
    }

    const terminalError = this.getTerminalWorkspaceError(snapshot)
    if (terminalError) {
      throw new Error(
        `Workspace provisioning failed: ${terminalError}. Retry from the workspace retry endpoint.`,
      )
    }

    return null
  }

  private async prepareProvisioning(
    taskId: string,
    repositoryId: string,
  ): Promise<{ plan: WorkspaceProvisionPlan; initializingTarget: ExecutionTarget | null }> {
    const repoConfig = await this.planService.getRepositoryConfig(repositoryId)
    const plan = await this.planService.createProvisionPlan(taskId, repoConfig)

    try {
      const initializingTarget = await this.provisioner.getInitializingTarget(
        taskId,
        repositoryId,
        plan,
      )
      return { plan, initializingTarget }
    } catch (error) {
      logger.warn(
        { taskId, error: error instanceof Error ? error.message : String(error) },
        'WorkspaceRuntimeManager: Failed to resolve initializing target',
      )
      return { plan, initializingTarget: null }
    }
  }

  private async acquireSetupLock(
    taskId: string,
    repositoryId: string,
    snapshot: WorkspaceRuntimeSnapshot | null,
    initializingTarget: ExecutionTarget | null,
  ): Promise<{ lock: WorkspaceLock | null; acquired: boolean }> {
    if (!initializingTarget) {
      logger.info(
        { taskId },
        'WorkspaceRuntimeManager: acquireSetupLock - no initializingTarget, skipping lock',
      )
      return { lock: null, acquired: true }
    }

    const setupLock: WorkspaceLock = {
      type: 'setup',
      owner: this.instanceId,
      acquiredAt: new Date(),
    }

    if (snapshot) {
      logger.info(
        { taskId, instanceId: this.instanceId, existingLocks: snapshot.locks },
        'WorkspaceRuntimeManager: acquireSetupLock - snapshot exists, trying to acquire lock',
      )
      const acquired = await this.runtimeStore.tryAcquireLock(taskId, setupLock, SETUP_LOCK_TTL_MS)
      logger.info(
        { taskId, acquired },
        'WorkspaceRuntimeManager: acquireSetupLock - tryAcquireLock result',
      )
      if (acquired) {
        await this.runtimeStore.update(taskId, (current) => ({
          ...current,
          executionTarget: initializingTarget,
          locks: this.mergeLocks(current.locks, setupLock),
        }))
        await this.emitEnvironmentEvent(taskId, 'initializing', 'not_created')
      }
      return { lock: setupLock, acquired }
    }

    logger.info(
      { taskId, instanceId: this.instanceId },
      'WorkspaceRuntimeManager: acquireSetupLock - no snapshot, trying to insert',
    )
    const initializingSnapshot = createWorkspaceSnapshotFromTarget(
      taskId,
      repositoryId,
      initializingTarget,
    )
    initializingSnapshot.locks = [setupLock]
    const acquired = await this.runtimeStore.tryInsert(initializingSnapshot)
    logger.info(
      { taskId, acquired },
      'WorkspaceRuntimeManager: acquireSetupLock - tryInsert result',
    )
    if (acquired) {
      await this.emitEnvironmentEvent(taskId, 'initializing', 'not_created')
    }
    return { lock: setupLock, acquired }
  }

  private async provisionAndPersist(
    taskId: string,
    repositoryId: string,
    plan: WorkspaceProvisionPlan,
    initializingTarget: ExecutionTarget | null,
  ): Promise<WorkspaceHandle> {
    logger.info(
      { taskId, repositoryId, hasInitializingTarget: !!initializingTarget },
      'WorkspaceRuntimeManager: Starting provisionAndPersist',
    )
    try {
      logger.info({ taskId }, 'WorkspaceRuntimeManager: Calling provisioner.provision()')
      const executionTarget = await this.provisioner.provision(taskId, repositoryId, plan)
      logger.info({ taskId }, 'WorkspaceRuntimeManager: provisioner.provision() completed')
      const snapshot = createWorkspaceSnapshotFromTarget(taskId, repositoryId, executionTarget)
      await this.runtimeStore.save(snapshot)
      await this.emitEnvironmentEvent(taskId, 'ready', 'initializing')

      const mode = isLocalExecutionTarget(executionTarget) ? 'local' : 'kubernetes'
      if (!isLocalExecutionTarget(executionTarget) || executionTarget.isAvailable !== false) {
        activeWorkspaces.labels({ mode }).inc()
      }

      logger.debug(
        {
          taskId,
          workspacePath: getWorkspacePathFromTarget(executionTarget),
        },
        'WorkspaceRuntimeManager: Workspace provisioned and snapshot saved',
      )

      return this.createEnrichedHandle(snapshot)
    } catch (error) {
      logger.error(
        { taskId, error: error instanceof Error ? error.message : String(error) },
        'WorkspaceRuntimeManager: provisionAndPersist failed',
      )
      const errorMessage = error instanceof Error ? error.message : String(error)
      if (initializingTarget && !isLocalExecutionTarget(initializingTarget)) {
        const errorTarget = {
          ...initializingTarget,
          podPhase: 'Failed' as const,
          lastError: errorMessage,
        }
        const errorSnapshot = createWorkspaceSnapshotFromTarget(taskId, repositoryId, errorTarget)
        await this.runtimeStore.save(errorSnapshot)
        logger.info({ taskId }, 'WorkspaceRuntimeManager: Saved error snapshot')
      }
      await this.emitEnvironmentEvent(taskId, 'error', 'initializing', errorMessage)

      throw error
    }
  }

  /**
   * Check if a snapshot is still valid
   */
  private async isSnapshotValid(snapshot: WorkspaceRuntimeSnapshot): Promise<boolean> {
    const isValid = await this.provisioner.validate(snapshot.executionTarget)

    if (!isValid) {
      logger.warn(
        {
          taskId: snapshot.taskId,
        },
        'WorkspaceRuntimeManager: Workspace snapshot invalid',
      )
    }

    return isValid
  }

  private mergeLocks(existing: WorkspaceLock[] | undefined, lock: WorkspaceLock): WorkspaceLock[] {
    const filtered = (existing ?? []).filter(
      (existingLock) => !(existingLock.type === lock.type && existingLock.owner === lock.owner),
    )
    return [...filtered, lock]
  }

  private getTerminalWorkspaceError(snapshot: WorkspaceRuntimeSnapshot): string | null {
    if (isLocalExecutionTarget(snapshot.executionTarget)) {
      return null
    }

    const target = snapshot.executionTarget
    if (target.lastError) {
      return target.lastError
    }

    if (
      target.podPhase &&
      ['Failed', 'Succeeded', 'Unknown', 'Terminating'].includes(target.podPhase)
    ) {
      return `Pod entered ${target.podPhase} phase`
    }

    return null
  }

  private hasActiveSetupLock(snapshot: WorkspaceRuntimeSnapshot): boolean {
    const now = Date.now()
    return (snapshot.locks ?? []).some(
      (lock) => lock.type === 'setup' && now - lock.acquiredAt.getTime() < SETUP_LOCK_TTL_MS,
    )
  }

  private async waitForProvisioning(
    taskId: string,
    repositoryId?: string,
  ): Promise<WorkspaceHandle> {
    const deadline = Date.now() + SETUP_WAIT_TIMEOUT_MS

    while (Date.now() < deadline) {
      const snapshot = await this.runtimeStore.load(taskId)
      if (!snapshot) {
        await this.sleep(SETUP_WAIT_INTERVAL_MS)
        continue
      }

      if (await this.isSnapshotValid(snapshot)) {
        return this.createEnrichedHandle(snapshot)
      }

      const terminalError = this.getTerminalWorkspaceError(snapshot)
      if (terminalError) {
        throw new Error(
          `Workspace provisioning failed: ${terminalError}. ` +
            'Retry from the workspace retry endpoint.',
        )
      }

      if (!this.hasActiveSetupLock(snapshot)) {
        return await this.ensureWorkspace(taskId, repositoryId)
      }

      await this.sleep(SETUP_WAIT_INTERVAL_MS)
    }

    throw new Error(`Timed out waiting for workspace provisioning for task ${taskId}`)
  }

  private async sleep(durationMs: number): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, durationMs))
  }

  /**
   * Stop agent for a task.
   * Used during workspace cleanup.
   */
  async stopAgentAndInvalidate(taskId: string): Promise<void> {
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      return
    }

    const driver = this.provisioner.createDriver(taskId, snapshot.executionTarget)
    try {
      await driver.stopAgent()
    } catch (error) {
      logger.warn(
        { taskId, error: error instanceof Error ? error.message : String(error) },
        'WorkspaceRuntimeManager: Error stopping agent during invalidation',
      )
    }
  }

  /**
   * Create enriched workspace handle from snapshot
   * Includes all runtime state and driver needed by consumers
   */
  private createEnrichedHandle(snapshot: WorkspaceRuntimeSnapshot): WorkspaceHandle {
    const isLocal = isLocalExecutionTarget(snapshot.executionTarget)
    const driver = this.provisioner.createDriver(snapshot.taskId, snapshot.executionTarget)
    return {
      taskId: snapshot.taskId,
      workspacePath: getWorkspacePathFromTarget(snapshot.executionTarget),
      repositoryId: snapshot.repositoryId,
      mode: isLocal ? 'local' : 'worker',
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
      driver,
    }
  }

  /**
   * Emit an environment status event
   */
  private async emitEnvironmentEvent(
    taskId: string,
    status: 'not_created' | 'initializing' | 'ready' | 'cleaning' | 'error',
    previousStatus: string,
    error?: string,
  ): Promise<void> {
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: 'environment',
      subType: status,
      taskId,
      timestamp: new Date(),
      isError: status === 'error',
      payload: {
        status,
        previousStatus,
        ...(error && { error }),
      },
    })
  }
}
