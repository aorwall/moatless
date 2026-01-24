import { randomUUID } from 'node:crypto'
import type { EnvironmentStatus, WorkspaceStatusResponse } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { activeWorkspaces } from '../../metrics'
import { RepositoryService } from '../../repositories/services/repository.service'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeRepository } from '../db/workspace-runtime.repository'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import type {
  WorkspaceProvisioner,
  WorkspaceStatusResult,
} from '../types/workspace-provisioner.types'
import {
  type AgentState,
  createWorkspaceSnapshotFromTarget,
  type ExecutionTarget,
  getWorkspacePathFromTarget,
  isDockerExecutionTarget,
  isKubernetesExecutionTarget,
  isWorkspaceAvailable as isTargetAvailable,
  type PreviewState,
  type WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'
import { ProvisionPlanFactory } from './provision-plan-factory.service'

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
  /** Per-task lock to prevent concurrent provisioning races */
  private provisioningLocks = new Map<string, Promise<WorkspaceHandle>>()

  constructor(
    @inject(WorkspaceRuntimeRepository) private readonly runtimeStore: WorkspaceRuntimeRepository,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject('WorkspaceProvisioner')
    private readonly provisioner: WorkspaceProvisioner,
    @inject(EventService) private readonly eventService: EventService,
    @inject(ProvisionPlanFactory) private readonly planFactory: ProvisionPlanFactory,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
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
      if (snapshot) {
        this.touchActivityIfStale(taskId, snapshot)
      }
      return existingHandle
    }

    // 2. Provisioning needed — use per-task lock to prevent concurrent provisioning
    const existingLock = this.provisioningLocks.get(taskId)
    if (existingLock) {
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Awaiting existing provisioning lock')
      return existingLock
    }

    const provisionPromise = this.doProvision(taskId, repositoryId, snapshot).finally(() => {
      this.provisioningLocks.delete(taskId)
    })
    this.provisioningLocks.set(taskId, provisionPromise)
    return provisionPromise
  }

  private async doProvision(
    taskId: string,
    repositoryId: string | undefined,
    snapshot: WorkspaceRuntimeSnapshot | null,
  ): Promise<WorkspaceHandle> {
    // Try to get repositoryId from: parameter → snapshot → task
    let repoId = repositoryId ?? snapshot?.repositoryId

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
    return await this.provisionAndPersist(taskId, repoId, plan)
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
   * - For Docker: checks container status
   *
   * Use this method when you need accurate real-time status (e.g., for status endpoints).
   * Use getSnapshot() when you just need the persisted state.
   *
   * Smart error preservation:
   * - If error exists and environment is recovering (ready/initializing), clear error
   * - If error exists and environment is still gone (stopped/not_created), preserve error status
   */
  async getValidatedSnapshot(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      return null
    }

    // Get live status from provisioner
    const liveStatus = await this.provisioner.getStatus(taskId, snapshot.executionTarget)

    // Merge live status into execution target with smart error handling
    const mergedTarget = this.mergeStatusIntoTarget(snapshot.executionTarget, liveStatus)

    return {
      ...snapshot,
      executionTarget: mergedTarget,
    }
  }

  /**
   * Explicitly mark workspace as active. Fire-and-forget.
   * Call this only for real user/agent interactions (preview access, agent events).
   */
  touchActivity(taskId: string, snapshot?: WorkspaceRuntimeSnapshot | null): void {
    if (snapshot) {
      this.touchActivityIfStale(taskId, snapshot)
      return
    }
    this.runtimeStore
      .load(taskId)
      .then((loaded) => {
        if (loaded) {
          this.touchActivityIfStale(taskId, loaded)
        }
      })
      .catch((err) => {
        logger.debug({ taskId, err }, 'Failed to load snapshot for activity touch')
      })
  }

  /**
   * Merge live status into execution target with smart error preservation
   *
   * Logic:
   * - If error exists and environment is recovering (ready/initializing), clear error
   * - If error exists and environment is still gone (stopped/not_created), preserve error status
   * - Otherwise, use live status
   */
  private mergeStatusIntoTarget(
    target: ExecutionTarget,
    liveStatus: WorkspaceStatusResult,
  ): ExecutionTarget {
    const hasExistingError = !!target.lastError
    const isRecovering = liveStatus.status === 'ready' || liveStatus.status === 'initializing'

    // If error exists and environment is recovering, clear error
    // If error exists and environment is still gone, preserve error status
    const shouldClearError = hasExistingError && isRecovering
    const shouldPreserveError = hasExistingError && !isRecovering

    // Preserve 'initializing' from snapshot when live status is 'not_created'
    // During provisioning, the pod/container may not exist yet — that's expected
    const shouldPreserveInitializing =
      target.status === 'initializing' && liveStatus.status === 'not_created'

    const resolvedStatus = shouldPreserveError
      ? 'error'
      : shouldPreserveInitializing
        ? 'initializing'
        : liveStatus.status

    return {
      ...target,
      status: resolvedStatus,
      lastHealthCheck: new Date(),
      lastError: shouldClearError ? undefined : target.lastError,
    }
  }

  /**
   * Get workspace path from runtime snapshot
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

    // All workspaces now use remote workers (Docker or K8s), so query the provisioner
    return await this.provisioner.isAgentRunning(taskId, snapshot.executionTarget)
  }

  /**
   * Delete workspace snapshot
   */
  async deleteSnapshot(taskId: string): Promise<void> {
    // Get snapshot to determine mode for metrics
    const snapshot = await this.runtimeStore.load(taskId)
    if (snapshot) {
      const mode = snapshot.executionTarget.mode
      if (isTargetAvailable(snapshot.executionTarget)) {
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

      // All workspaces use remote workers, cleanup handled by provisioner
      await this.provisioner.cleanup(taskId, snapshot.executionTarget)

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
   * Check if workspace is available and ready for use
   * Checks if the execution target (K8s pod or Docker container) is running and ready
   */
  async isWorkspaceAvailable(taskId: string): Promise<boolean> {
    const snapshot = await this.runtimeStore.load(taskId)
    if (!snapshot) {
      return false
    }
    return isTargetAvailable(snapshot.executionTarget)
  }

  /**
   * Complete workspace cleanup - orchestrates all cleanup operations
   *
   * Workflow:
   * 1. Stop any running agents
   * 2. Clean up workspace artifacts (clones, worktrees)
   * 3. Delete workspace snapshot
   *
   * This is the single entry point for full workspace cleanup.
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
        // Continue cleanup even if agent stop fails
      }

      // 2. Clean up workspace artifacts (clones, worktrees, etc.)
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
        // Continue cleanup even if artifact cleanup fails
      }

      // 3. Delete workspace snapshot
      await this.deleteSnapshot(taskId)
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Workspace snapshot deleted')

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
   * Get workspace status for API response
   * Maps runtime snapshot to WorkspaceStatusResponse format
   */
  async getWorkspaceStatus(taskId: string): Promise<WorkspaceStatusResponse> {
    // Validate task exists first
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Get workspace snapshot with live environment validation
    const snapshot = await this.getValidatedSnapshot(taskId)
    if (!snapshot) {
      // No snapshot exists yet - workspace hasn't been provisioned
      return {
        workspaceState: 'missing',
        environmentStatus: 'not_created',
        agentState: 'none',
        capabilities: {
          fileOperations: false,
          commandExecution: false,
          serverManagement: false,
        },
        isRunning: false,
        workspaceReady: false,
        canExecute: false,
      }
    }

    // Map snapshot to status response
    const repository = await this.repoService.getRepositoryById(snapshot.repositoryId)

    // Check if workspace is available
    const workspaceAvailable = isTargetAvailable(snapshot.executionTarget)

    // Determine environment status
    const getEnvironmentStatus = (): EnvironmentStatus => {
      if (snapshot.executionTarget.lastError) {
        return 'error'
      }
      return snapshot.executionTarget.status ?? 'not_created'
    }

    const environmentStatus = getEnvironmentStatus()

    const status: WorkspaceStatusResponse = {
      workspaceState: 'exists',
      environmentStatus,
      environmentError: snapshot.executionTarget.lastError,
      agentState: snapshot.agent?.state === 'running' ? 'running' : 'none',
      capabilities: {
        fileOperations: workspaceAvailable,
        commandExecution: workspaceAvailable,
        serverManagement:
          workspaceAvailable &&
          Boolean(
            repository?.previewConfig?.serverStartCommands &&
              repository.previewConfig.serverStartCommands.length > 0,
          ),
      },
      isRunning:
        environmentStatus === 'ready' &&
        (snapshot.preview.status === 'started' || snapshot.agent?.state === 'running'),
      workspaceReady: environmentStatus === 'ready',
      canExecute: environmentStatus === 'ready',
    }

    return status
  }

  /**
   * Retry workspace provisioning after a failure
   * Clears the error state and provisions a new workspace immediately
   */
  async retryWorkspace(taskId: string): Promise<{ success: boolean; message: string }> {
    // Validate task exists
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Clear the error state (deletes pod and snapshot)
    await this.clearWorkspaceError(taskId)

    // Provision new workspace immediately
    try {
      await this.ensureWorkspace(taskId, task.repositoryId)
      return {
        success: true,
        message: 'Workspace reprovisioned successfully.',
      }
    } catch (error) {
      return {
        success: false,
        message: `Failed to reprovision workspace: ${(error as Error).message}`,
      }
    }
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

    const statusResult = await this.provisioner.getStatus(snapshot.taskId, snapshot.executionTarget)

    if (statusResult.status === 'ready') {
      logger.debug({ taskId: snapshot.taskId }, 'WorkspaceRuntimeManager: Using existing workspace')
      return this.createEnrichedHandle(snapshot)
    }

    // Log INFO with actual status - not an error, just informational
    logger.info(
      {
        taskId: snapshot.taskId,
        status: statusResult.status,
        containerName: statusResult.containerName,
        podName: statusResult.podName,
      },
      'WorkspaceRuntimeManager: Existing workspace not ready, will provision',
    )

    const terminalError = await this.getTerminalWorkspaceError(snapshot)
    if (terminalError) {
      throw new Error(
        `Workspace provisioning failed: ${terminalError}. Retry from the workspace retry endpoint.`,
      )
    }

    return null
  }

  private async provisionAndPersist(
    taskId: string,
    repositoryId: string,
    plan: WorkspaceProvisionPlan,
  ): Promise<WorkspaceHandle> {
    try {
      const executionTarget = await this.provisioner.provision(taskId, plan)
      const snapshot = createWorkspaceSnapshotFromTarget(taskId, repositoryId, executionTarget)
      await this.runtimeStore.save(snapshot)
      await this.emitEnvironmentEvent(taskId, 'ready', 'not_created')

      const mode = executionTarget.mode
      if (isTargetAvailable(executionTarget)) {
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
      const errorMessage = error instanceof Error ? error.message : String(error)
      await this.emitEnvironmentEvent(taskId, 'error', 'not_created', errorMessage)
      throw error
    }
  }

  /**
   * Check if workspace has a terminal error that prevents retry.
   *
   * For workspaces with cached errors, this method checks the current
   * container/pod status before returning the error. If the container/pod
   * no longer exists or is now running, the error is cleared to allow
   * fresh provisioning.
   */
  private getTerminalWorkspaceError(snapshot: WorkspaceRuntimeSnapshot): Promise<string | null> {
    const target = snapshot.executionTarget

    // Handle cached errors by checking current status
    if (target.lastError) {
      return this.handleCachedError(snapshot)
    }

    // Check for terminal states without cached error
    return Promise.resolve(this.getTerminalStateError(target))
  }

  /**
   * Handle cached error by checking live status
   *
   * Uses getStatus() to check current environment state and determine
   * if the error should be cleared for retry or returned as terminal.
   */
  private async handleCachedError(snapshot: WorkspaceRuntimeSnapshot): Promise<string | null> {
    const target = snapshot.executionTarget
    logger.debug(
      { taskId: snapshot.taskId, lastError: target.lastError },
      'WorkspaceRuntimeManager: Checking status for cached error',
    )

    const liveStatus = await this.provisioner.getStatus(snapshot.taskId, target)

    // Check if resource no longer exists - clear error for retry
    if (liveStatus.status === 'not_created' || liveStatus.status === 'stopped') {
      const statusMsg = isDockerExecutionTarget(target)
        ? liveStatus.status === 'not_created'
          ? 'Container not found'
          : 'Container stopped'
        : liveStatus.status === 'not_created'
          ? 'Pod not found'
          : 'Pod stopped'
      logger.info(
        { taskId: snapshot.taskId },
        `WorkspaceRuntimeManager: ${statusMsg}, clearing cached error to allow retry`,
      )
      await this.runtimeStore.delete(snapshot.taskId)
      return null
    }

    // Check if resource is now running - clear error
    if (liveStatus.status === 'ready') {
      const statusMsg = isDockerExecutionTarget(target)
        ? 'Container is now running'
        : 'Pod is now running'
      logger.info(
        { taskId: snapshot.taskId },
        `WorkspaceRuntimeManager: ${statusMsg}, clearing cached error`,
      )
      // Update snapshot with cleared error and new status
      const updatedTarget: ExecutionTarget = {
        ...target,
        status: 'ready',
        lastHealthCheck: new Date(),
        lastError: undefined,
      }
      const updatedSnapshot = createWorkspaceSnapshotFromTarget(
        snapshot.taskId,
        snapshot.repositoryId,
        updatedTarget,
      )
      await this.runtimeStore.save(updatedSnapshot)
      return null
    }

    // Resource still exists but is in a failed state - return cached error
    return target.lastError ?? null
  }

  /**
   * Get error for terminal state without cached error
   * Uses unified status field to determine terminal states
   *
   * Terminal states that return an error:
   * - 'error': Container/pod crashed or failed
   *
   * Non-terminal states that allow re-provisioning:
   * - 'not_created': Container/pod doesn't exist
   * - 'stopped': Container/pod was stopped (not an error)
   * - 'removing': Container/pod being cleaned up
   */
  private getTerminalStateError(target: ExecutionTarget): string | null {
    // Only 'error' status is a true terminal error that requires user intervention
    // Other states like 'stopped', 'not_created', 'removing' allow automatic recovery
    if (target.status === 'error' && target.lastError) {
      return target.lastError
    }

    // Also check K8s-specific terminal phases for backwards compatibility
    if (isKubernetesExecutionTarget(target)) {
      const terminalPhases = ['Failed', 'Unknown']
      if (target.podPhase && terminalPhases.includes(target.podPhase)) {
        return `Pod entered ${target.podPhase} phase`
      }
    }

    return null
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
    const driver = this.provisioner.createDriver(snapshot.taskId, snapshot.executionTarget)
    return {
      taskId: snapshot.taskId,
      workspacePath: getWorkspacePathFromTarget(snapshot.executionTarget),
      repositoryId: snapshot.repositoryId,
      mode: 'worker', // All workspaces now use remote workers (Docker or K8s)
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
    status: 'not_created' | 'initializing' | 'ready' | 'stopped' | 'removing' | 'error',
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

  // ========================================
  // Activity Tracking
  // ========================================

  private getActivityThrottleMs(): number {
    return Number(process.env['WORKSPACE_ACTIVITY_THROTTLE_MS']) || 5 * 60 * 1000
  }

  /** Update lastActivityAt in DB if stale (older than throttle). Fire-and-forget. */
  private touchActivityIfStale(taskId: string, snapshot: WorkspaceRuntimeSnapshot): void {
    const now = Date.now()
    const lastActivity = snapshot.lastActivityAt?.getTime() ?? 0
    if (now - lastActivity > this.getActivityThrottleMs()) {
      const timestamp = new Date()
      this.runtimeStore.touchActivity(taskId, timestamp).catch((err) => {
        logger.debug({ taskId, err }, 'Failed to update lastActivityAt')
      })
    }
  }
}
