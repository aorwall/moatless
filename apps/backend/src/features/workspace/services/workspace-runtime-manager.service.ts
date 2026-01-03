import crypto from 'node:crypto'
import path from 'node:path'
import { type RepositoryConfig, WorkspaceError } from '@moatless/types'
import { inject, injectable, optional } from 'inversify'
import { KubernetesService } from '../../../services/kubernetes.service'
import {
  activeWorkspaces,
  driverCacheHitTotal,
  kubernetesPodReadinessDurationSeconds,
  kubernetesPodTotal,
  workspaceProvisionDurationSeconds,
  workspaceProvisionTotal,
} from '../../../services/metrics'
import { SettingsService } from '../../../services/settings.service'
import { getDataDir } from '../../../utils/data-dir'
import { pathExists } from '../../../utils/filesystem'
import { logger } from '../../../utils/logger'
import { GitProviderTokenService } from '../../auth/services/git-provider-token.service'
import { WorkerTokenService } from '../../auth/services/worker-token.service'
import { ClaudeTokenService } from '../../claude/services/claude-token.service'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeRepository } from '../db/workspace-runtime.repository'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceHandle } from '../types/workspace-handle.types'
import type { WorkspaceProvisioner } from '../types/workspace-provisioner.types'
import {
  type AgentState,
  createKubernetesWorkspaceSnapshot,
  createWorkspaceSnapshot,
  getWorkspacePathFromTarget,
  isLocalExecutionTarget,
  type KubernetesExecutionTarget,
  type KubernetesWorkspaceConfig,
  type LocalExecutionTarget,
  type PreviewState,
  type WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'
import { WorkspaceMaterializer, WorkspacePathMissingError } from './workspace-materializer.service'

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
 * - Driver creation and caching (per-task)
 * - Returns enriched WorkspaceHandle with runtime state and driver
 *
 * Design principle: Consumers should NOT access RuntimeRepository directly.
 * All state access/updates flow through this manager.
 * Environment drivers are always obtained through WorkspaceHandle.driver.
 */
@injectable()
export class WorkspaceRuntimeManager {
  /**
   * Cache of per-task driver instances
   * Key: taskId, Value: driver instance
   */
  private drivers = new Map<string, BaseEnvironmentDriver>()

  constructor(
    @inject(WorkspaceRuntimeRepository) private readonly runtimeStore: WorkspaceRuntimeRepository,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceMaterializer) private readonly materializer: WorkspaceMaterializer,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(SettingsService) private readonly settingsService: SettingsService,
    @inject(KubernetesService) @optional() private readonly kubernetesService: KubernetesService,
    @inject(GitProviderTokenService)
    private readonly gitProviderTokenService: GitProviderTokenService,
    @inject(ClaudeTokenService)
    private readonly claudeTokenService: ClaudeTokenService,
    @inject(WorkerTokenService)
    private readonly workerTokenService: WorkerTokenService,
    @inject('WorkspaceProvisioner')
    private readonly provisioner: WorkspaceProvisioner,
  ) {}

  /**
   * Ensure workspace is provisioned and ready
   * Returns enriched handle with current runtime state
   * Handles gracefully when workspace path doesn't exist (returns unavailable workspace)
   *
   * Routes to Kubernetes provisioning if execution mode is 'kubernetes'
   */
  async ensureWorkspace(taskId: string, repositoryId?: string): Promise<WorkspaceHandle> {
    logger.debug({ taskId, repositoryId }, 'WorkspaceRuntimeManager: Ensuring workspace')
    const startTime = Date.now()

    // Check execution mode - route to Kubernetes if enabled
    const executionMode = this.settingsService.getWorkspaceExecutionMode()
    if (executionMode === 'kubernetes') {
      return this.ensureKubernetesWorkspace(taskId, repositoryId, startTime)
    }

    // 1. Try to load existing snapshot
    let snapshot = await this.runtimeStore.load(taskId)

    //  2. If snapshot exists and is valid, return existing workspace
    if (snapshot && (await this.isSnapshotValid(snapshot))) {
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Using existing workspace from snapshot')
      return await this.createEnrichedHandle(snapshot)
    }

    // 3. Provision new workspace (local mode)
    logger.info(
      { taskId, repositoryId },
      'WorkspaceRuntimeManager: Provisioning new local workspace',
    )

    const repoId = repositoryId ?? snapshot?.repositoryId
    if (!repoId) {
      throw new Error(`Repository ID required to provision workspace for task ${taskId}`)
    }

    const repoConfig = await this.repoService.getRepositoryById(repoId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repoId}`)
    }

    const plan = await this.createProvisionPlan(taskId, repoConfig)

    try {
      const handle = await this.materializer.applyPlan(taskId, plan)

      // 4. Create and save snapshot
      snapshot = createWorkspaceSnapshot(taskId, repoId, handle.workspacePath, true)
      // createWorkspaceSnapshot always creates a local execution target
      const localTarget = snapshot.executionTarget as LocalExecutionTarget
      localTarget.lastValidatedAt = new Date()
      await this.runtimeStore.save(snapshot)

      // Track successful provisioning metrics
      const durationSeconds = (Date.now() - startTime) / 1000
      workspaceProvisionTotal.labels({ mode: plan.mode, status: 'success' }).inc()
      workspaceProvisionDurationSeconds.labels({ mode: plan.mode }).observe(durationSeconds)
      activeWorkspaces.labels({ mode: 'local' }).inc()

      logger.debug(
        {
          taskId,
          workspacePath: handle.workspacePath,
        },
        'WorkspaceRuntimeManager: Workspace provisioned and snapshot saved',
      )

      return await this.createEnrichedHandle(snapshot)
    } catch (error) {
      // Handle workspace path missing gracefully
      if (error instanceof WorkspacePathMissingError) {
        logger.warn(
          {
            taskId,
            path: error.path,
            error: error.message,
          },
          'WorkspaceRuntimeManager: Workspace path missing, creating unavailable workspace',
        )

        // Create snapshot with workspace marked as unavailable
        snapshot = createWorkspaceSnapshot(taskId, repoId, error.path, false)
        // createWorkspaceSnapshot always creates a local execution target
        const localTarget = snapshot.executionTarget as LocalExecutionTarget
        localTarget.lastError = error.message
        localTarget.lastValidatedAt = new Date()
        await this.runtimeStore.save(snapshot)

        // Track error metrics
        workspaceProvisionTotal.labels({ mode: 'direct', status: 'error' }).inc()

        return await this.createEnrichedHandle(snapshot)
      }

      // Track error metrics for other errors
      workspaceProvisionTotal.labels({ mode: 'direct', status: 'error' }).inc()

      // Re-throw other errors
      throw error
    }
  }

  /**
   * Ensure Kubernetes workspace is provisioned and ready
   * Creates a pod for task execution if one doesn't exist
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex orchestration with multiple pod lifecycle states
  private async ensureKubernetesWorkspace(
    taskId: string,
    repositoryId?: string,
    startTime?: number,
  ): Promise<WorkspaceHandle> {
    const provisionStartTime = startTime ?? Date.now()
    logger.debug({ taskId, repositoryId }, 'WorkspaceRuntimeManager: Ensuring Kubernetes workspace')

    if (!this.kubernetesService) {
      throw new Error('Kubernetes service not available')
    }

    // 1. Try to load existing snapshot
    let snapshot = await this.runtimeStore.load(taskId)

    // 2. If snapshot exists with a running Kubernetes target, check if pod is still valid
    if (snapshot && !isLocalExecutionTarget(snapshot.executionTarget)) {
      // Query K8s directly for pod IP (on-demand, not from snapshot)
      const podIp = await this.kubernetesService.getPodIp(taskId)
      if (podIp) {
        const isHealthy = await this.kubernetesService.checkPodHealth(podIp)
        if (isHealthy) {
          logger.debug({ taskId, podIp }, 'WorkspaceRuntimeManager: Using existing Kubernetes pod')
          return await this.createEnrichedHandle(snapshot)
        }
        logger.warn(
          { taskId, podIp },
          'WorkspaceRuntimeManager: Kubernetes pod unhealthy, reprovisioning',
        )
      }
    }

    // 2.5 Check if pod already exists in Kubernetes (handles Pending phase race condition)
    // This prevents creating duplicate pods when multiple requests arrive before pod is ready
    const existingPod = await this.kubernetesService.findExistingPod(taskId)
    if (existingPod) {
      logger.info(
        {
          taskId,
          podName: existingPod.podName,
          phase: existingPod.phase,
          podIp: existingPod.podIp,
        },
        'WorkspaceRuntimeManager: Found existing pod in Kubernetes',
      )

      if (existingPod.phase === 'Pending' || existingPod.phase === 'Running') {
        // Pod already exists, wait for it instead of creating a new one
        logger.info(
          { taskId, podName: existingPod.podName },
          'WorkspaceRuntimeManager: Waiting for existing pod to become ready',
        )

        try {
          const podStatus = await this.kubernetesService.waitForPodReady(taskId)

          // Create/update snapshot with the pod info (podIp derived on-demand, not stored)
          const repoId = repositoryId ?? snapshot?.repositoryId
          const k8sSettings = this.settingsService.getKubernetesSettings()

          // Get repositoryCloneUrl from existing snapshot's k8s target if available
          const existingK8sTarget =
            snapshot && !isLocalExecutionTarget(snapshot.executionTarget)
              ? (snapshot.executionTarget as KubernetesExecutionTarget)
              : null

          const snapshotConfig: KubernetesWorkspaceConfig = {
            taskId,
            repositoryId: repoId ?? '', // Will be updated if needed
            podName: existingPod.podName,
            podNamespace: k8sSettings.namespace,
            podPhase: podStatus.phase,
            repositoryCloneUrl: existingK8sTarget?.repositoryCloneUrl ?? '',
          }
          const newSnapshot = createKubernetesWorkspaceSnapshot(snapshotConfig)
          await this.runtimeStore.save(newSnapshot)

          logger.info(
            { taskId, podName: existingPod.podName },
            'WorkspaceRuntimeManager: Using existing pod that is now ready',
          )
          return await this.createEnrichedHandle(newSnapshot)
        } catch (waitError) {
          // WorkspaceError indicates permanent failure (ImagePullBackOff, ErrImagePull, etc.)
          // These won't recover with a retry, so propagate them immediately
          if (waitError instanceof WorkspaceError) {
            logger.error(
              { taskId, podName: existingPod.podName, err: waitError },
              'WorkspaceRuntimeManager: Pod failed with permanent error',
            )
            throw waitError
          }

          // For other errors (transient), delete pod and retry
          logger.warn(
            { taskId, podName: existingPod.podName, err: waitError as Error },
            'WorkspaceRuntimeManager: Existing pod failed to become ready, will delete and retry',
          )
          // Pod failed to become ready, delete it and continue to create a new one
          await this.kubernetesService.deletePod(taskId)
        }
      } else if (existingPod.phase === 'Failed' || existingPod.phase === 'Succeeded') {
        // Delete completed/failed pod before creating a new one
        logger.info(
          { taskId, podName: existingPod.podName, phase: existingPod.phase },
          'WorkspaceRuntimeManager: Deleting completed/failed pod',
        )
        await this.kubernetesService.deletePod(taskId)
      }
    }

    // 3. Get repository info for pod provisioning
    const repoId = repositoryId ?? snapshot?.repositoryId
    if (!repoId) {
      throw new Error(`Repository ID required to provision Kubernetes workspace for task ${taskId}`)
    }

    const repoConfig = await this.repoService.getRepositoryById(repoId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repoId}`)
    }

    // 4. Get git clone URL based on repository type
    const gitCloneUrl = await this.resolveGitCloneUrl(taskId, repoConfig)
    const gitBranch: string | undefined = undefined

    // 5. Create pod
    const k8sSettings = this.settingsService.getKubernetesSettings()

    logger.info(
      { taskId, repositoryName: repoConfig.name, gitCloneUrl },
      'WorkspaceRuntimeManager: Creating Kubernetes pod',
    )

    // Get GitHub token for authenticated git operations
    const githubToken = await this.gitProviderTokenService.getTokenForGitOperations(gitCloneUrl)

    // Get Anthropic API key for Claude Code agent
    const anthropicApiKey = process.env['ANTHROPIC_API_KEY']

    // Get Claude OAuth token for Claude Code CLI authentication
    // Priority: 1. User's database token (OAuth/PAT), 2. Environment variable
    const { SessionContext } = await import('../../auth/services/session-context')
    const userId = SessionContext.getCurrentUserId()
    logger.info(
      { taskId, hasSession: SessionContext.hasSession(), userId },
      'Retrieving Claude token for pod creation',
    )

    let claudeOAuthToken = await this.claudeTokenService.getTokenForAgent()
    logger.info(
      { taskId, hasToken: !!claudeOAuthToken, fallbackToEnv: !claudeOAuthToken, userId },
      'Claude token retrieval result',
    )

    if (!claudeOAuthToken) {
      claudeOAuthToken = process.env['CLAUDE_CODE_OAUTH_TOKEN'] ?? null
      if (claudeOAuthToken) {
        logger.info({ taskId }, 'Using CLAUDE_CODE_OAUTH_TOKEN from environment variable')
      }
    }

    // Fail fast if no Claude authentication is available
    // Note: "placeholder" is not a valid ANTHROPIC_API_KEY
    const hasValidAnthropicKey = anthropicApiKey && anthropicApiKey !== 'placeholder'
    if (!(claudeOAuthToken || hasValidAnthropicKey)) {
      throw new Error(
        'Cannot create worker pod: No Claude authentication configured. ' +
          'Please connect a Claude account (Settings → Claude) or set ANTHROPIC_API_KEY environment variable.',
      )
    }

    // Generate JWT token for worker authentication
    // This token allows the worker to authenticate back to the main API as this user
    const session = SessionContext.getCurrentSession()
    if (!session) {
      throw new Error(
        'Cannot create worker pod: No authenticated user session. ' +
          'Please log in before creating tasks.',
      )
    }

    const workerAuthToken = await this.workerTokenService.generateToken({
      userId: session.userId,
      taskId,
      userLogin: session.userLogin,
      userEmail: session.userEmail ?? undefined,
      groupId: session.groupId,
    })

    logger.info(
      { taskId, userId: session.userId },
      'WorkspaceRuntimeManager: Generated worker auth token',
    )

    const podConfig: Parameters<typeof this.kubernetesService.createPod>[1] = {
      repositoryName: repoConfig.name,
      repositoryCloneUrl: gitCloneUrl,
      workerApiUrl: k8sSettings.workerApiUrl || '',
      workerAuthToken,
    }
    if (gitBranch) {
      podConfig.repositoryBranch = gitBranch
    }
    if (githubToken) {
      podConfig.githubToken = githubToken
    }
    // Only include ANTHROPIC_API_KEY if it's a valid key (not "placeholder")
    // and we don't have an OAuth token (OAuth takes priority)
    if (anthropicApiKey && anthropicApiKey !== 'placeholder' && !claudeOAuthToken) {
      podConfig.anthropicApiKey = anthropicApiKey
    }
    if (claudeOAuthToken) {
      podConfig.claudeOAuthToken = claudeOAuthToken
    }
    // Pass DATABASE_URL for event fallback when HTTP delivery fails
    const databaseUrl = process.env['DATABASE_URL']
    if (databaseUrl) {
      podConfig.databaseUrl = databaseUrl
    }

    // 5.1 Save initializing snapshot BEFORE pod creation so frontend can show loading state
    const initializingSnapshotConfig: KubernetesWorkspaceConfig = {
      taskId,
      repositoryId: repoId,
      podName: `moatless-${taskId.slice(0, 8)}`,
      podNamespace: k8sSettings.namespace,
      repositoryCloneUrl: gitCloneUrl,
      podPhase: 'Pending', // Indicates provisioning in progress
    }
    if (gitBranch) {
      initializingSnapshotConfig.repositoryBranch = gitBranch
    }
    const initializingSnapshot = createKubernetesWorkspaceSnapshot(initializingSnapshotConfig)
    await this.runtimeStore.save(initializingSnapshot)
    logger.debug({ taskId }, 'WorkspaceRuntimeManager: Saved initializing snapshot')

    let podName: string | undefined
    let podNamespace: string | undefined

    try {
      const createResult = await this.kubernetesService.createPod(taskId, podConfig)
      podName = createResult.podName
      podNamespace = createResult.podNamespace

      // Track pod creation
      kubernetesPodTotal.labels({ status: 'created' }).inc()

      // 6. Wait for pod to be ready
      logger.info({ taskId, podName }, 'WorkspaceRuntimeManager: Waiting for pod to be ready')
      const podStatus = await this.kubernetesService.waitForPodReady(taskId)

      // 7. Create and save Kubernetes snapshot (podIp derived on-demand, not stored)
      const snapshotConfig: KubernetesWorkspaceConfig = {
        taskId,
        repositoryId: repoId,
        podName,
        podNamespace,
        podPhase: podStatus.phase,
        repositoryCloneUrl: gitCloneUrl,
      }
      if (gitBranch) {
        snapshotConfig.repositoryBranch = gitBranch
      }
      snapshot = createKubernetesWorkspaceSnapshot(snapshotConfig)
      await this.runtimeStore.save(snapshot)

      // Track successful Kubernetes provisioning metrics
      const durationSeconds = (Date.now() - provisionStartTime) / 1000
      workspaceProvisionTotal.labels({ mode: 'kubernetes', status: 'success' }).inc()
      workspaceProvisionDurationSeconds.labels({ mode: 'kubernetes' }).observe(durationSeconds)
      activeWorkspaces.labels({ mode: 'kubernetes' }).inc()
      kubernetesPodTotal.labels({ status: 'ready' }).inc()
      kubernetesPodReadinessDurationSeconds.observe(durationSeconds)

      logger.info({ taskId, podName }, 'WorkspaceRuntimeManager: Kubernetes workspace provisioned')

      return await this.createEnrichedHandle(snapshot)
    } catch (error) {
      // Track error metrics
      workspaceProvisionTotal.labels({ mode: 'kubernetes', status: 'error' }).inc()
      kubernetesPodTotal.labels({ status: 'failed' }).inc()

      // Save error snapshot so getWorkspaceStatus can report the failure
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { taskId, error: errorMessage, podName },
        'WorkspaceRuntimeManager: Kubernetes pod provisioning failed',
      )

      const errorSnapshotConfig: Parameters<typeof createKubernetesWorkspaceSnapshot>[0] = {
        taskId,
        repositoryId: repoId,
        podName: podName ?? `moatless-${taskId.slice(0, 8)}`,
        podNamespace: podNamespace ?? k8sSettings.namespace,
        repositoryCloneUrl: gitCloneUrl,
        podPhase: 'Failed',
        lastError: errorMessage,
      }
      if (gitBranch) {
        errorSnapshotConfig.repositoryBranch = gitBranch
      }
      const errorSnapshot = createKubernetesWorkspaceSnapshot(errorSnapshotConfig)
      await this.runtimeStore.save(errorSnapshot)

      throw error
    }
  }

  /**
   * Get workspace runtime snapshot
   */
  async getSnapshot(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    return await this.runtimeStore.load(taskId)
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

    // Validate and update execution target state based on environment type
    if (isLocalExecutionTarget(snapshot.executionTarget)) {
      // Local: check if workspace path exists
      const localTarget = snapshot.executionTarget as LocalExecutionTarget
      const exists = await pathExists(localTarget.workspacePath)
      localTarget.isAvailable = exists
      if (!exists) {
        localTarget.lastError = 'Workspace path no longer exists'
      }
    } else if (this.kubernetesService) {
      // Kubernetes: check if pod actually exists in the cluster
      const k8sTarget = snapshot.executionTarget as KubernetesExecutionTarget
      const existingPod = await this.kubernetesService.findExistingPod(taskId)

      if (existingPod) {
        // Update with live pod status (podIp is derived on-demand, not stored)
        k8sTarget.podPhase = existingPod.phase
        // Clear any stale error if pod is now healthy
        if (existingPod.phase === 'Running' && k8sTarget.lastError) {
          k8sTarget.lastError = undefined
        }
      } else {
        // Pod no longer exists in Kubernetes - this is not an error,
        // just means workspace needs to be provisioned
        k8sTarget.podPhase = 'NotFound'
        // Don't set lastError - pod not existing is a normal state
      }
    }

    return snapshot
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

    // For Kubernetes targets, query the worker pod
    if (!isLocalExecutionTarget(snapshot.executionTarget)) {
      // Query K8s directly for pod IP (on-demand, not from snapshot)
      const podIp = await this.kubernetesService.getPodIp(taskId)
      if (!podIp) {
        return false // No pod IP = pod not ready
      }

      try {
        const workerUrl = `http://${podIp}:8001/agent-status`
        const response = await fetch(workerUrl, {
          signal: AbortSignal.timeout(5000), // 5 second timeout
        })

        if (!response.ok) {
          logger.debug(
            { taskId, podIp, status: response.status },
            'Worker agent-status returned non-OK response',
          )
          return false
        }

        const data = (await response.json()) as { state: string }
        const isRunning = data.state === 'running'

        logger.debug(
          { taskId, podIp, workerState: data.state, isRunning },
          'Checked worker agent status',
        )

        return isRunning
      } catch (error) {
        // Worker unreachable = not running
        logger.debug(
          { taskId, podIp, err: error },
          'Worker unreachable, treating agent as not running',
        )
        return false
      }
    }

    // For local targets, trust the database state (no separate worker to query)
    // Local agents run in the same process, so if the state says running,
    // we trust it (cleanup should have reset it if the agent crashed)
    return snapshot.agent?.state === 'running'
  }

  /**
   * Delete workspace snapshot
   */
  async deleteSnapshot(taskId: string): Promise<void> {
    // Get snapshot to determine mode for metrics
    const snapshot = await this.runtimeStore.load(taskId)
    if (snapshot) {
      const mode = isLocalExecutionTarget(snapshot.executionTarget) ? 'local' : 'kubernetes'
      activeWorkspaces.labels({ mode }).dec()
    }

    // Invalidate cached driver
    this.invalidateDriver(taskId)

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

    // If Kubernetes target, try to delete the failed pod
    if (!isLocalExecutionTarget(snapshot.executionTarget) && this.kubernetesService) {
      const k8sTarget = snapshot.executionTarget
      logger.debug({ taskId, podName: k8sTarget.podName }, 'Deleting failed Kubernetes pod')
      try {
        await this.kubernetesService.deletePod(taskId)
      } catch (error) {
        // Pod might already be gone, log and continue
        logger.debug(
          { taskId, error: (error as Error).message },
          'Failed to delete pod (may already be deleted)',
        )
      }
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
   * 3. Delegates to WorkspaceMaterializer for physical cleanup
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

      // 2. For Kubernetes workspaces, delete the pod
      if (!isLocalExecutionTarget(snapshot.executionTarget) && this.kubernetesService) {
        const k8sTarget = snapshot.executionTarget
        logger.info(
          { taskId, podName: k8sTarget.podName },
          'WorkspaceRuntimeManager: Deleting Kubernetes pod',
        )
        try {
          await this.kubernetesService.deletePod(taskId)
        } catch (error) {
          logger.warn(
            { taskId, error: (error as Error).message },
            'WorkspaceRuntimeManager: Failed to delete pod during cleanup (may already be deleted)',
          )
        }
        // No local artifacts to clean up for Kubernetes workspaces
        return
      }

      // 3. Get repository configuration (local workspaces only)
      const repoConfig = await this.repoService.getRepositoryById(snapshot.repositoryId)
      if (!repoConfig) {
        logger.warn(
          { taskId, repositoryId: snapshot.repositoryId },
          'Repository not found, cannot reconstruct provision plan for cleanup',
        )
        return
      }

      // 3. Reconstruct provision plan (same logic as ensureWorkspace)
      const plan = await this.createProvisionPlan(taskId, repoConfig)

      // 4. Delegate to materializer for physical cleanup
      await this.materializer.cleanupWorkspace(taskId, plan)

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

  /**
   * Create workspace provision plan
   */
  private async createProvisionPlan(
    taskId: string,
    repoConfig: RepositoryConfig,
  ): Promise<WorkspaceProvisionPlan> {
    // Template repositories should never be used directly for tasks
    // They are only used to create local repositories
    if (repoConfig.kind === 'template') {
      throw new Error(
        'Template repositories cannot be used directly for tasks. Create a local repository from the template first.',
      )
    }

    // Determine provision mode based on repository configuration
    const mode = this.determineProvisionMode(repoConfig)

    // Load task to get targetBranch for git push operations
    const task = await this.taskRepository.get(taskId)
    const targetBranch = task?.targetBranch

    // Determine working branch:
    // - If targetBranch is set (gitPushEnabled=true), use it as the working branch
    // - For worktree mode without targetBranch, use workspace-{taskId}
    // - For other modes without targetBranch, use default branch (undefined)
    const branch = targetBranch || (mode === 'worktree' ? `workspace-${taskId}` : undefined)

    const plan: WorkspaceProvisionPlan = {
      mode,
      branch,
      targetBranch,
      setupMetadataHash: this.computeSetupHash(repoConfig),
    }

    switch (mode) {
      case 'direct':
        // Use repository path directly (local only)
        if (repoConfig.kind === 'local') {
          plan.reusePath = repoConfig.path
        } else {
          throw new Error('Direct mode only supported for local repositories')
        }
        break

      case 'worktree':
        // Create worktree in workspace directory (local only)
        if (repoConfig.kind === 'local') {
          plan.reusePath = repoConfig.path
          plan.localTargetPath = this.getWorktreePath(repoConfig, taskId)
        } else {
          throw new Error('Worktree mode only supported for local repositories')
        }
        break

      case 'clone':
        // Clone to new directory
        if (repoConfig.kind === 'remote') {
          // For remote repos, clone from URL (no reusePath needed)
          plan.remoteUrl = repoConfig.remote.url
          plan.provider = repoConfig.remote.provider
          plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        } else if (repoConfig.kind === 'local') {
          // For local repos, clone from local path
          plan.reusePath = repoConfig.path
          plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        }
        break
    }

    logger.debug(
      {
        taskId,
        mode,
        branch: branch || 'default',
        targetBranch: targetBranch || 'none',
      },
      'WorkspaceRuntimeManager: Created provision plan',
    )

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
        // Use worktree mode if enabled, otherwise direct
        return repoConfig.useGitWorktrees ? 'worktree' : 'direct'

      case 'template':
        // Template repositories should never reach here due to early check
        throw new Error('Template repositories cannot be used directly for tasks')

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
    if (repoConfig.kind !== 'local') {
      throw new Error('Worktree path only supported for local repositories')
    }

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
    let repoName: string

    if (repoConfig.kind === 'local') {
      repoName = path.basename(repoConfig.path)
    } else if (repoConfig.kind === 'remote') {
      // For remote repos, use repository name from config
      repoName = repoConfig.name
    } else if (repoConfig.kind === 'template') {
      // Templates should never reach here due to early validation
      throw new Error('Template repositories cannot be used directly for tasks')
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
        metadata = {
          kind: repoConfig.kind,
          path: repoConfig.path,
          codeExecution: repoConfig.codeExecution,
          useGitWorktrees: repoConfig.useGitWorktrees,
        }
        break

      case 'remote':
      case 'template':
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
   * Resolve the git clone URL for Kubernetes workspace provisioning
   * Handles remote repos directly and local repos created from templates
   */
  private async resolveGitCloneUrl(taskId: string, repoConfig: RepositoryConfig): Promise<string> {
    if (repoConfig.kind === 'remote') {
      return repoConfig.remote.url
    }

    if (repoConfig.kind === 'local') {
      // For local repos created from templates, use the template's remote URL
      const task = await this.taskRepository.getInternal(taskId)
      if (task?.templateId) {
        const templateRepo = await this.repoService.getRepositoryById(task.templateId)
        if (templateRepo?.kind === 'template') {
          logger.info(
            { taskId, templateId: task.templateId, gitCloneUrl: templateRepo.remote.url },
            'WorkspaceRuntimeManager: Using template remote URL for vibe task',
          )
          return templateRepo.remote.url
        }
        throw new Error(
          `Template repository not found: ${task.templateId}. ` +
            'Local repositories without templates are not supported in Kubernetes mode.',
        )
      }
      throw new Error(
        'Kubernetes execution mode requires remote repository. ' +
          'Local repositories (without template) are not supported in Kubernetes mode.',
      )
    }

    throw new Error('Template repositories cannot be used directly for tasks')
  }

  /**
   * Check if a snapshot is still valid
   */
  private async isSnapshotValid(snapshot: WorkspaceRuntimeSnapshot): Promise<boolean> {
    // For Kubernetes execution targets, check pod status instead
    if (!isLocalExecutionTarget(snapshot.executionTarget)) {
      // TODO: Implement Kubernetes pod validation
      // For now, consider Kubernetes targets valid if they exist
      return true
    }

    // Check if workspace path still exists (local mode only)
    const workspacePath = snapshot.executionTarget.workspacePath
    const isValid = await this.materializer.validateWorkspace(workspacePath)

    if (!isValid) {
      logger.warn(
        {
          taskId: snapshot.taskId,
          workspacePath,
        },
        'WorkspaceRuntimeManager: Workspace snapshot invalid - path no longer exists',
      )
    }

    return isValid
  }

  /**
   * Get or create the appropriate environment driver for a task
   * Creates driver instances on first access and caches them for reuse
   *
   * IMPORTANT: Validates cached driver health before returning.
   * This detects stale drivers pointing to dead pods/missing workspaces.
   */
  private async getOrCreateDriver(
    snapshot: WorkspaceRuntimeSnapshot,
  ): Promise<BaseEnvironmentDriver> {
    // Check cache first
    const cached = this.drivers.get(snapshot.taskId)
    if (cached) {
      // Validate cached driver is still healthy
      const isHealthy = await this.provisioner.isHealthy(snapshot.executionTarget)
      if (isHealthy) {
        driverCacheHitTotal.labels({ result: 'hit_healthy' }).inc()
        return cached
      }
      // Stale driver - invalidate and recreate
      driverCacheHitTotal.labels({ result: 'hit_stale' }).inc()
      logger.info(
        { taskId: snapshot.taskId },
        'WorkspaceRuntimeManager: Cached driver unhealthy, recreating',
      )
      this.invalidateDriver(snapshot.taskId)
    }

    // Create driver via provisioner (always required)
    driverCacheHitTotal.labels({ result: 'miss' }).inc()
    const driver = this.provisioner.createDriver(snapshot.taskId, snapshot.executionTarget)

    // Cache and return
    this.drivers.set(snapshot.taskId, driver)

    logger.debug(
      {
        taskId: snapshot.taskId,
        driverType: isLocalExecutionTarget(snapshot.executionTarget) ? 'local' : 'kubernetes',
      },
      'WorkspaceRuntimeManager: Created and cached driver instance',
    )

    return driver
  }

  /**
   * Invalidate cached driver for a task
   * Called when workspace is reprovisioned (e.g., pod recreation)
   */
  invalidateDriver(taskId: string): void {
    if (this.drivers.has(taskId)) {
      this.drivers.delete(taskId)
      logger.debug({ taskId }, 'WorkspaceRuntimeManager: Invalidated cached driver')
    }
  }

  /**
   * Stop agent and cleanup cached driver for a task
   * Used during workspace cleanup
   */
  async stopAgentAndInvalidate(taskId: string): Promise<void> {
    const driver = this.drivers.get(taskId)
    if (driver) {
      try {
        await driver.stopAgent()
      } catch (error) {
        logger.warn(
          { taskId, error: error instanceof Error ? error.message : String(error) },
          'WorkspaceRuntimeManager: Error stopping agent during invalidation',
        )
      }
      this.drivers.delete(taskId)
    }
  }

  /**
   * Create enriched workspace handle from snapshot
   * Includes all runtime state and driver needed by consumers
   */
  private async createEnrichedHandle(snapshot: WorkspaceRuntimeSnapshot): Promise<WorkspaceHandle> {
    const isLocal = isLocalExecutionTarget(snapshot.executionTarget)
    const driver = await this.getOrCreateDriver(snapshot)
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
}
