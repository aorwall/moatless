import { randomUUID } from 'node:crypto'
import { inject, injectable, optional } from 'inversify'
import { type RedisLockService, RedisLockServiceSymbol } from '../../../services/redis-lock.service'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { activeWorkspaces } from '../../metrics'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceSandboxRepository } from '../db/workspace-sandbox.repository'
import type { SandboxRecord } from '../db/workspace-sandbox.schema'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import type {
  WorkspaceProvisioner,
  WorkspaceStatusResult,
} from '../types/workspace-provisioner.types'
import {
  type ExecutionTarget,
  getSandboxUrl,
  getWorkspacePathFromTarget,
  isDockerExecutionTarget,
  isWorkspaceAvailable as isTargetAvailable,
} from '../types/workspace-runtime.types'
import { ProvisionPlanFactory } from './provision-plan-factory.service'

/**
 * WorkspaceRuntimeManager - Low-level runtime state manager
 *
 * Owns all workspace provisioning, runtime state management, and driver lifecycle.
 * Uses WorkspaceSandboxRepository as the sole persistence layer.
 *
 * Agent state and preview state are NOT persisted — they are queried on-demand
 * from the sandbox. Only the execution target is persisted (needed for provisioner operations).
 *
 * Responsibilities:
 * - Workspace provisioning and validation
 * - Sandbox record CRUD operations
 * - State updates for execution target
 * - Driver creation (per-task)
 * - Returns enriched WorkspaceHandle with runtime state and driver
 */
/** TTL for distributed provisioning lock (2 minutes) */
const PROVISION_LOCK_TTL_MS = 120_000
/** How long to wait for another replica's provisioning to complete */
const PROVISION_WAIT_TIMEOUT_MS = 90_000

@injectable()
export class WorkspaceRuntimeManager {
  /** Per-task lock to prevent concurrent provisioning races within this replica */
  private provisioningLocks = new Map<string, Promise<WorkspaceHandle>>()

  constructor(
    @inject(WorkspaceSandboxRepository) private readonly sandboxStore: WorkspaceSandboxRepository,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject('WorkspaceProvisioner')
    private readonly provisioner: WorkspaceProvisioner,
    @inject(EventService) private readonly eventService: EventService,
    @inject(ProvisionPlanFactory) private readonly planFactory: ProvisionPlanFactory,
    @inject(RedisLockServiceSymbol)
    @optional()
    private readonly redisLock: RedisLockService | null,
  ) {}

  /**
   * Ensure workspace is provisioned and ready
   * Returns enriched handle with current runtime state
   *
   * Uses a two-level locking strategy:
   * 1. In-memory lock (provisioningLocks) - prevents races within this replica
   * 2. Redis distributed lock - prevents races across replicas
   */
  async ensureWorkspace(taskId: string, repositoryId?: string): Promise<WorkspaceHandle> {
    logger.debug({ taskId, repositoryId }, 'WorkspaceRuntimeManager: Ensuring workspace')

    // 1. Try to load existing sandbox record
    const record = await this.sandboxStore.load(taskId)

    const existingHandle = await this.tryGetExistingHandle(record)
    if (existingHandle) {
      if (record) {
        this.touchActivityIfStale(taskId, record)
      }
      return existingHandle
    }

    // 2. Provisioning needed — use per-task lock to prevent concurrent provisioning within this replica
    const existingLock = this.provisioningLocks.get(taskId)
    if (existingLock) {
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Awaiting existing provisioning lock')
      return existingLock
    }

    const provisionPromise = this.doProvisionWithDistributedLock(taskId, repositoryId).finally(
      () => {
        this.provisioningLocks.delete(taskId)
      },
    )
    this.provisioningLocks.set(taskId, provisionPromise)
    return provisionPromise
  }

  /**
   * Provision workspace with distributed lock to prevent races across replicas
   */
  private async doProvisionWithDistributedLock(
    taskId: string,
    repositoryId: string | undefined,
  ): Promise<WorkspaceHandle> {
    const lockKey = `workspace:provision:${taskId}`

    // If RedisLockService is not available, fall back to single-replica behavior
    if (!this.redisLock?.isHealthy()) {
      logger.debug(
        { taskId },
        'WorkspaceRuntimeManager: Redis lock not available, using local-only provisioning',
      )
      return this.doProvision(taskId, repositoryId)
    }

    // Attempt to acquire distributed lock
    const lockToken = await this.redisLock.acquireLock(lockKey, PROVISION_LOCK_TTL_MS)

    if (!lockToken) {
      // Another replica is provisioning - wait for it to complete
      logger.info(
        { taskId },
        'WorkspaceRuntimeManager: Another replica is provisioning, waiting for completion',
      )

      const unlocked = await this.redisLock.waitForUnlock(
        lockKey,
        PROVISION_WAIT_TIMEOUT_MS,
        1000, // Poll every second
      )

      if (!unlocked) {
        throw new Error(
          `Workspace provisioning timed out waiting for another replica to complete (task: ${taskId})`,
        )
      }

      // After waiting, check if workspace is now available
      const record = await this.sandboxStore.load(taskId)
      const existingHandle = await this.tryGetExistingHandle(record)
      if (existingHandle) {
        logger.info({ taskId }, 'WorkspaceRuntimeManager: Workspace provisioned by another replica')
        return existingHandle
      }

      // If still not available, another replica may have failed - retry with lock
      logger.info(
        { taskId },
        'WorkspaceRuntimeManager: Other replica may have failed, retrying provisioning',
      )
      return this.doProvisionWithDistributedLock(taskId, repositoryId)
    }

    try {
      // Double-check after acquiring lock (another replica may have just finished)
      const record = await this.sandboxStore.load(taskId)
      const existingHandle = await this.tryGetExistingHandle(record)
      if (existingHandle) {
        logger.debug({ taskId }, 'WorkspaceRuntimeManager: Workspace appeared after acquiring lock')
        return existingHandle
      }

      // Proceed with actual provisioning
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Acquired distributed lock, provisioning')
      return await this.doProvision(taskId, repositoryId)
    } finally {
      // Always release the lock
      const released = await this.redisLock.releaseLock(lockKey, lockToken)
      if (!released) {
        logger.warn(
          { taskId },
          'WorkspaceRuntimeManager: Failed to release distributed lock (may have expired)',
        )
      }
    }
  }

  private async doProvision(
    taskId: string,
    repositoryId: string | undefined,
  ): Promise<WorkspaceHandle> {
    // Try to get repositoryId from: parameter → task
    let repoId = repositoryId

    // If still not found, lookup task and get repositoryId from task object
    if (!repoId) {
      const task = await this.taskRepository.get(taskId)
      if (task) {
        repoId = task.repositoryId
        logger.debug(
          { taskId, repositoryId: repoId },
          'WorkspaceRuntimeManager: Retrieved repositoryId from task',
        )
      }
    }

    if (!repoId) {
      throw new Error(
        `Repository ID required to provision workspace for task ${taskId}. ` +
          'Task not found or has no repositoryId.',
      )
    }

    const plan = await this.planFactory.createPlan(taskId, repoId)
    return await this.provisionAndPersist(taskId, plan)
  }

  /**
   * Get sandbox record by task ID.
   */
  async getSandboxRecord(taskId: string): Promise<SandboxRecord | null> {
    return await this.sandboxStore.load(taskId)
  }

  /**
   * Get execution target for a task, or null if no sandbox exists.
   */
  async getExecutionTarget(taskId: string): Promise<ExecutionTarget | null> {
    const record = await this.sandboxStore.load(taskId)
    return record?.executionTarget ?? null
  }

  /**
   * Get sandbox URL for a task, or null if not available.
   */
  async getSandboxUrl(taskId: string): Promise<string | null> {
    const target = await this.getExecutionTarget(taskId)
    if (!target) {
      return null
    }
    return getSandboxUrl(target)
  }

  /**
   * Get workspace handle without provisioning or validation.
   * Returns null if no sandbox record exists.
   */
  async getExistingWorkspaceHandle(taskId: string): Promise<WorkspaceHandle | null> {
    const record = await this.sandboxStore.load(taskId)
    if (!record) {
      return null
    }

    return this.createEnrichedHandle(record)
  }

  /**
   * Get sandbox record with live environment validation.
   * Returns the record with execution target updated from live status.
   */
  async getValidatedRecord(taskId: string): Promise<SandboxRecord | null> {
    const record = await this.sandboxStore.load(taskId)
    if (!record) {
      return null
    }

    const liveStatus = await this.provisioner.getStatus(taskId, record.executionTarget)

    return {
      ...record,
      executionTarget: {
        ...record.executionTarget,
        status: liveStatus.status,
        lastHealthCheck: new Date(),
        lastError: liveStatus.lastError ?? record.executionTarget.lastError,
      },
    }
  }

  /**
   * Explicitly mark workspace as active. Fire-and-forget.
   */
  touchActivity(taskId: string): void {
    this.sandboxStore
      .load(taskId)
      .then((record) => {
        if (record) {
          this.touchActivityIfStale(taskId, record)
        }
      })
      .catch((err) => {
        logger.debug({ taskId, err }, 'Failed to load sandbox record for activity touch')
      })
  }

  /**
   * Check if an agent is actually running by querying the worker.
   */
  async isAgentActuallyRunning(taskId: string): Promise<boolean> {
    const record = await this.sandboxStore.load(taskId)
    if (!record) {
      return false
    }
    return await this.provisioner.isAgentRunning(taskId, record.executionTarget)
  }

  /**
   * Delete sandbox record and clean up metrics.
   */
  async deleteSandbox(taskId: string): Promise<void> {
    const record = await this.sandboxStore.load(taskId)
    if (record) {
      const mode = record.executionTarget.mode
      const liveStatus = await this.provisioner.getStatus(taskId, record.executionTarget)
      if (liveStatus.status === 'ready') {
        activeWorkspaces.labels({ mode }).dec()
      }
    }

    await this.sandboxStore.unregister(taskId)
  }

  /**
   * Clear workspace error state and prepare for retry
   */
  async clearWorkspaceError(taskId: string): Promise<void> {
    logger.info({ taskId }, 'WorkspaceRuntimeManager: Clearing workspace error state')

    const record = await this.sandboxStore.load(taskId)
    if (!record) {
      logger.debug({ taskId }, 'No sandbox record found to clear')
      return
    }

    try {
      await this.provisioner.stopSandbox(taskId, record.executionTarget)
    } catch (error) {
      logger.debug(
        { taskId, error: (error as Error).message },
        'Failed to stop sandbox while clearing workspace error',
      )
    }

    // Delete the sandbox record to allow fresh provisioning
    await this.sandboxStore.unregister(taskId)

    logger.info({ taskId }, 'WorkspaceRuntimeManager: Workspace error state cleared')
  }

  /**
   * Clean up workspace artifacts before deleting sandbox record
   */
  async cleanupWorkspaceArtifacts(taskId: string): Promise<void> {
    logger.debug({ taskId }, 'WorkspaceRuntimeManager: Cleaning up workspace artifacts')

    try {
      const record = await this.sandboxStore.load(taskId)
      if (!record) {
        logger.debug({ taskId }, 'No sandbox record found, skipping artifact cleanup')
        return
      }

      await this.provisioner.cleanup(taskId, record.executionTarget)

      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Workspace artifacts cleaned up')
    } catch (error) {
      logger.error(
        { err: error as Error, taskId },
        'WorkspaceRuntimeManager: Failed to clean up workspace artifacts',
      )
    }
  }

  /**
   * Check if workspace is available and ready for use
   */
  async isWorkspaceAvailable(taskId: string): Promise<boolean> {
    const record = await this.sandboxStore.load(taskId)
    if (!record) {
      return false
    }
    const liveStatus = await this.provisioner.getStatus(taskId, record.executionTarget)
    return liveStatus.status === 'ready'
  }

  /**
   * Complete workspace cleanup - orchestrates all cleanup operations
   */
  async deleteWorkspace(taskId: string): Promise<void> {
    logger.info({ taskId }, 'WorkspaceRuntimeManager: Starting complete workspace cleanup')

    try {
      // 1. Stop any running agents
      try {
        await this.stopAgentAndInvalidate(taskId)
        logger.debug({ taskId }, 'WorkspaceRuntimeManager: Agent stopped and driver invalidated')
      } catch (error) {
        logger.warn(
          {
            taskId,
            error: error instanceof Error ? error.message : String(error),
          },
          'WorkspaceRuntimeManager: Failed to stop agent during cleanup',
        )
      }

      // 2. Clean up workspace artifacts
      try {
        await this.cleanupWorkspaceArtifacts(taskId)
        logger.debug({ taskId }, 'WorkspaceRuntimeManager: Workspace artifacts cleaned up')
      } catch (error) {
        logger.warn(
          {
            taskId,
            error: error instanceof Error ? error.message : String(error),
          },
          'WorkspaceRuntimeManager: Failed to clean up workspace artifacts',
        )
      }

      // 3. Delete sandbox record
      await this.deleteSandbox(taskId)
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Sandbox record deleted')

      logger.info({ taskId }, 'WorkspaceRuntimeManager: Complete workspace cleanup finished')
    } catch (error) {
      logger.error(
        { err: error as Error, taskId },
        'WorkspaceRuntimeManager: Failed to complete workspace cleanup',
      )
      throw error
    }
  }

  /**
   * Notify the sandbox that new messages are available.
   * Fire-and-forget — notification is best-effort.
   */
  async notifySandbox(taskId: string): Promise<void> {
    const record = await this.sandboxStore.load(taskId)
    if (!record) {
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: No sandbox record, skipping notification')
      return
    }

    try {
      const driver = this.provisioner.createDriver(taskId, record.executionTarget)
      await driver.notifyMessages()
    } catch (error) {
      logger.debug(
        { taskId, err: error instanceof Error ? error.message : String(error) },
        'WorkspaceRuntimeManager: Failed to notify sandbox (best-effort)',
      )
    }
  }

  // ========================================
  // Private Helper Methods
  // ========================================

  private async tryGetExistingHandle(
    record: SandboxRecord | null,
  ): Promise<WorkspaceHandle | null> {
    if (!record) {
      return null
    }

    const statusResult = await this.provisioner.getStatus(record.taskId, record.executionTarget)

    if (statusResult.status === 'ready') {
      logger.debug({ taskId: record.taskId }, 'WorkspaceRuntimeManager: Using existing workspace')
      // Emit ready event if the stored state wasn't already ready (e.g. recovered from error)
      if (record.executionTarget.status !== 'ready') {
        await this.emitEnvironmentEvent(record.taskId, 'ready')
      }
      return this.createEnrichedHandle(record)
    }

    logger.info(
      {
        taskId: record.taskId,
        status: statusResult.status,
        containerName: statusResult.containerName,
        podName: statusResult.podName,
      },
      'WorkspaceRuntimeManager: Existing workspace not ready, will provision',
    )

    const terminalError = await this.getTerminalWorkspaceError(record, statusResult)
    if (terminalError) {
      throw new Error(
        `Workspace provisioning failed: ${terminalError}. Retry from the workspace retry endpoint.`,
      )
    }

    return null
  }

  private async provisionAndPersist(
    taskId: string,
    plan: WorkspaceProvisionPlan,
  ): Promise<WorkspaceHandle> {
    const initTarget = this.provisioner.createInitializingTarget(taskId, plan)
    const mode = initTarget.mode

    try {
      // Save initializing sandbox record immediately
      await this.sandboxStore.register(taskId, mode, initTarget)

      await this.emitEnvironmentEvent(taskId, 'initializing')

      // Provision workspace (blocking - can take 30+ seconds for K8s)
      const executionTarget = await this.provisioner.provision(taskId, plan)

      // Update sandbox record with final execution target
      await this.sandboxStore.register(taskId, executionTarget.mode, executionTarget)

      // Only emit 'ready' if the provisioner returned a ready target.
      // When status is 'initializing', the sandbox controller publishes
      // environment:ready via Redis when the pod actually becomes healthy.
      if (executionTarget.status !== 'initializing') {
        await this.emitEnvironmentEvent(taskId, 'ready')
      }

      if (isTargetAvailable(executionTarget)) {
        activeWorkspaces.labels({ mode }).inc()
      }

      logger.debug(
        {
          taskId,
          workspacePath: getWorkspacePathFromTarget(executionTarget),
        },
        'WorkspaceRuntimeManager: Workspace provisioned and sandbox record saved',
      )

      const record = await this.sandboxStore.load(taskId)
      return this.createEnrichedHandle(record!)
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)

      // Update sandbox record with error state
      const errorTarget: ExecutionTarget = {
        ...initTarget,
        status: 'error',
        lastError: errorMessage,
      }
      await this.sandboxStore.register(taskId, mode, errorTarget)
      await this.sandboxStore.setError(taskId, errorMessage).catch((err) => {
        logger.debug({ taskId, err }, 'Failed to set sandbox error')
      })

      await this.emitEnvironmentEvent(taskId, 'error', errorMessage)
      throw error
    }
  }

  /**
   * Check if workspace has a terminal error that prevents retry.
   */
  private getTerminalWorkspaceError(
    record: SandboxRecord,
    liveStatus: WorkspaceStatusResult,
  ): Promise<string | null> {
    if (record.executionTarget.lastError) {
      return this.handleCachedError(record)
    }

    return Promise.resolve(this.getTerminalStateError(liveStatus))
  }

  private async handleCachedError(record: SandboxRecord): Promise<string | null> {
    const target = record.executionTarget
    logger.debug(
      { taskId: record.taskId, lastError: target.lastError },
      'WorkspaceRuntimeManager: Checking status for cached error',
    )

    const liveStatus = await this.provisioner.getStatus(record.taskId, target)

    if (liveStatus.status === 'not_created' || liveStatus.status === 'stopped') {
      const statusMsg = isDockerExecutionTarget(target)
        ? liveStatus.status === 'not_created'
          ? 'Container not found'
          : 'Container stopped'
        : liveStatus.status === 'not_created'
          ? 'Pod not found'
          : 'Pod stopped'
      logger.info(
        { taskId: record.taskId },
        `WorkspaceRuntimeManager: ${statusMsg}, clearing cached error to allow retry`,
      )
      await this.sandboxStore.unregister(record.taskId)
      return null
    }

    if (liveStatus.status === 'ready') {
      const statusMsg = isDockerExecutionTarget(target)
        ? 'Container is now running'
        : 'Pod is now running'
      logger.info(
        { taskId: record.taskId },
        `WorkspaceRuntimeManager: ${statusMsg}, clearing cached error`,
      )
      const updatedTarget: ExecutionTarget = {
        ...target,
        status: 'ready',
        lastHealthCheck: new Date(),
        lastError: undefined,
      }
      await this.sandboxStore.updateExecutionTarget(record.taskId, updatedTarget)
      await this.sandboxStore.clearError(record.taskId)
      await this.emitEnvironmentEvent(record.taskId, 'ready')
      return null
    }

    return target.lastError ?? null
  }

  private getTerminalStateError(liveStatus: WorkspaceStatusResult): string | null {
    if (liveStatus.status === 'error' && liveStatus.lastError) {
      return liveStatus.lastError
    }
    return null
  }

  /**
   * Stop agent for a task.
   */
  async stopAgentAndInvalidate(taskId: string): Promise<void> {
    const record = await this.sandboxStore.load(taskId)
    if (!record) {
      return
    }

    const driver = this.provisioner.createDriver(taskId, record.executionTarget)
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
   * Create enriched workspace handle from sandbox record.
   * Agent/preview state are set to defaults — consumers should query sandbox for live state.
   */
  private createEnrichedHandle(record: SandboxRecord): WorkspaceHandle {
    const driver = this.provisioner.createDriver(record.taskId, record.executionTarget)
    return {
      taskId: record.taskId,
      workspacePath: getWorkspacePathFromTarget(record.executionTarget),
      repositoryId: '', // Not stored in sandbox; consumers get from task record
      mode: 'worker',
      preview: {
        status: 'stopped',
        needsSetup: true,
      },
      agent: {
        state: 'idle' as const,
      },
      driver,
    }
  }

  /**
   * Emit an environment status event
   */
  private async emitEnvironmentEvent(
    taskId: string,
    status: 'not_created' | 'initializing' | 'ready' | 'stopped' | 'removing' | 'error',
    error?: string,
  ): Promise<void> {
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: 'environment',
      subType: status,
      taskId,
      timestamp: new Date(),
      isError: status === 'error',
      payload: error ? { error } : {},
    })
  }

  // ========================================
  // Activity Tracking
  // ========================================

  private getActivityThrottleMs(): number {
    return Number(process.env['WORKSPACE_ACTIVITY_THROTTLE_MS']) || 5 * 60 * 1000
  }

  /** Update lastActivityAt in DB if stale (older than throttle). Fire-and-forget. */
  private touchActivityIfStale(taskId: string, record: SandboxRecord): void {
    const now = Date.now()
    const lastActivity = record.lastActivityAt?.getTime() ?? 0
    if (now - lastActivity > this.getActivityThrottleMs()) {
      const timestamp = new Date()
      this.sandboxStore.touchActivity(taskId, timestamp).catch((err) => {
        logger.debug({ taskId, err }, 'Failed to update lastActivityAt')
      })
    }
  }

  /**
   * Get the sandbox repository for direct access by background services
   */
  getSandboxStore(): WorkspaceSandboxRepository {
    return this.sandboxStore
  }
}
