import { randomUUID } from 'node:crypto'
import { inject, injectable, optional } from 'inversify'
import { type RedisLockService, RedisLockServiceSymbol } from '../../../services/redis-lock.service'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { activeWorkspaces } from '../../metrics'
import { TaskRepository } from '../../tasks/db/task.repository'
import { SandboxControllerProvisioner } from '../provisioners/sandbox-controller-provisioner'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import {
  type ExecutionTarget,
  getWorkspacePathFromTarget,
  isWorkspaceAvailable as isTargetAvailable,
} from '../types/workspace-runtime.types'
import { ProvisionPlanFactory } from './provision-plan-factory.service'

/**
 * WorkspaceRuntimeManager - Low-level runtime state manager
 *
 * Owns all workspace provisioning, runtime state management, and driver lifecycle.
 * The sandbox controller API is the sole source of truth for workspace state.
 * No local DB persistence — execution targets are constructed deterministically
 * from taskId (service names are deterministic).
 *
 * Responsibilities:
 * - Workspace provisioning and validation
 * - State queries via sandbox controller API
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
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(SandboxControllerProvisioner)
    private readonly provisioner: SandboxControllerProvisioner,
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

    // 1. Check sandbox controller for existing workspace
    const existingHandle = await this.tryGetExistingHandle(taskId)
    if (existingHandle) {
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
      const existingHandle = await this.tryGetExistingHandle(taskId)
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
      const existingHandle = await this.tryGetExistingHandle(taskId)
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
    return await this.provisionWorkspace(taskId, plan)
  }

  /**
   * Get workspace status from the sandbox controller.
   * Returns status and optional error information.
   */
  async getWorkspaceStatus(
    taskId: string,
  ): Promise<{ exists: boolean; status: string; error?: string }> {
    const target = this.buildExecutionTarget(taskId)
    const liveStatus = await this.provisioner.getStatus(taskId, target)
    const exists = liveStatus.status !== 'not_created' && liveStatus.status !== 'error'
    const result: { exists: boolean; status: string; error?: string } = {
      exists,
      status: liveStatus.status,
    }
    if (liveStatus.lastError !== undefined) {
      result.error = liveStatus.lastError
    }
    return result
  }

  /**
   * Get execution target for a task, constructed deterministically from taskId.
   */
  getExecutionTarget(taskId: string): ExecutionTarget {
    return this.buildExecutionTarget(taskId)
  }

  /**
   * Check if an agent is actually running by querying the sandbox.
   */
  async isAgentActuallyRunning(taskId: string): Promise<boolean> {
    const target = this.buildExecutionTarget(taskId)
    return await this.provisioner.isAgentRunning(taskId, target)
  }

  /**
   * Delete sandbox and clean up metrics.
   */
  async deleteSandbox(taskId: string): Promise<void> {
    const target = this.buildExecutionTarget(taskId)
    const liveStatus = await this.provisioner.getStatus(taskId, target)
    if (liveStatus.status === 'ready') {
      activeWorkspaces.labels({ mode: 'kubernetes' }).dec()
    }
  }

  /**
   * Clear workspace error state and prepare for retry
   */
  async clearWorkspaceError(taskId: string): Promise<void> {
    logger.info({ taskId }, 'WorkspaceRuntimeManager: Clearing workspace error state')

    try {
      const target = this.buildExecutionTarget(taskId)
      await this.provisioner.stopSandbox(taskId, target)
    } catch (error) {
      logger.debug(
        { taskId, error: (error as Error).message },
        'Failed to stop sandbox while clearing workspace error',
      )
    }

    logger.info({ taskId }, 'WorkspaceRuntimeManager: Workspace error state cleared')
  }

  /**
   * Clean up workspace artifacts via sandbox controller
   */
  async cleanupWorkspaceArtifacts(taskId: string): Promise<void> {
    logger.debug({ taskId }, 'WorkspaceRuntimeManager: Cleaning up workspace artifacts')

    try {
      const target = this.buildExecutionTarget(taskId)
      await this.provisioner.cleanup(taskId, target)

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
    const target = this.buildExecutionTarget(taskId)
    const liveStatus = await this.provisioner.getStatus(taskId, target)
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

      // 3. Update metrics
      await this.deleteSandbox(taskId)
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Sandbox metrics updated')

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
    try {
      const target = this.buildExecutionTarget(taskId)
      const driver = this.provisioner.createDriver(taskId, target)
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

  /**
   * Build an ExecutionTarget deterministically from taskId.
   * Service names are deterministic: moatless-ws-{sanitized-taskId}
   * No DB lookup needed.
   */
  private buildExecutionTarget(taskId: string): ExecutionTarget {
    const namespace = process.env['K8S_NAMESPACE'] || process.env['SANDBOX_NAMESPACE'] || 'default'

    // Deterministic service name (must match Rust's generate_service_name)
    const sanitized = taskId
      .toLowerCase()
      .replace(/[^a-z0-9-]/g, '-')
      .substring(0, 49)
    const serviceName = `moatless-ws-${sanitized}`.substring(0, 63)

    return {
      mode: 'kubernetes',
      podNamespace: namespace,
      serviceName,
      repositoryCloneUrl: '', // Not needed for status/driver operations
      sandboxPort: 8001,
    }
  }

  private async tryGetExistingHandle(taskId: string): Promise<WorkspaceHandle | null> {
    const target = this.buildExecutionTarget(taskId)
    const statusResult = await this.provisioner.getStatus(taskId, target)

    if (statusResult.status === 'ready') {
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Using existing workspace')
      return this.createEnrichedHandle(taskId, target)
    }

    if (statusResult.status === 'not_created' || statusResult.status === 'stopped') {
      // No workspace exists or it's stopped — needs provisioning
      return null
    }

    if (statusResult.status === 'initializing') {
      // Still initializing — not ready yet, but exists
      logger.info(
        { taskId, status: statusResult.status, podName: statusResult.podName },
        'WorkspaceRuntimeManager: Existing workspace not ready, will provision',
      )
      return null
    }

    // Error state
    if (statusResult.status === 'error' && statusResult.lastError) {
      throw new Error(
        `Workspace provisioning failed: ${statusResult.lastError}. Retry from the workspace retry endpoint.`,
      )
    }

    logger.info(
      { taskId, status: statusResult.status, podName: statusResult.podName },
      'WorkspaceRuntimeManager: Existing workspace not ready, will provision',
    )
    return null
  }

  private async provisionWorkspace(
    taskId: string,
    plan: WorkspaceProvisionPlan,
  ): Promise<WorkspaceHandle> {
    this.provisioner.createInitializingTarget(taskId, plan)

    try {
      await this.emitEnvironmentEvent(taskId, 'initializing')

      // Provision workspace (blocking - can take 30+ seconds for K8s)
      const executionTarget = await this.provisioner.provision(taskId, plan)

      // Only emit 'ready' if the provisioner returned a ready target.
      // When status is 'initializing', the sandbox controller publishes
      // environment:ready via Redis when the pod actually becomes healthy.
      if (executionTarget.status !== 'initializing') {
        await this.emitEnvironmentEvent(taskId, 'ready')
      }

      if (isTargetAvailable(executionTarget)) {
        activeWorkspaces.labels({ mode: 'kubernetes' }).inc()
      }

      logger.debug(
        {
          taskId,
          workspacePath: getWorkspacePathFromTarget(executionTarget),
        },
        'WorkspaceRuntimeManager: Workspace provisioned',
      )

      return this.createEnrichedHandle(taskId, executionTarget)
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)

      await this.emitEnvironmentEvent(taskId, 'error', errorMessage)
      throw error
    }
  }

  /**
   * Stop agent for a task.
   */
  async stopAgentAndInvalidate(taskId: string): Promise<void> {
    const target = this.buildExecutionTarget(taskId)
    const driver = this.provisioner.createDriver(taskId, target)
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
   * Create enriched workspace handle from execution target.
   * Agent/preview state are set to defaults — consumers should query sandbox for live state.
   */
  private createEnrichedHandle(taskId: string, target: ExecutionTarget): WorkspaceHandle {
    const driver = this.provisioner.createDriver(taskId, target)
    return {
      taskId,
      workspacePath: getWorkspacePathFromTarget(target),
      repositoryId: '', // Not stored in sandbox; consumers get from task record
      mode: 'sandbox',
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
}
