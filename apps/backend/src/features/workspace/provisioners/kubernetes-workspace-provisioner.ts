import type { EnvironmentStatus, RepositoryConfig } from '@moatless/types'
import { WorkspaceError } from '@moatless/types'
import { inject, injectable, optional } from 'inversify'
import * as jose from 'jose'
import { KubernetesService, type PodCreationConfig } from '../../../services/kubernetes.service'
import { SettingsService } from '../../../services/settings.service'
import { logger } from '../../../utils/logger'
import {
  kubernetesPodReadinessDurationSeconds,
  kubernetesPodTotal,
  provisionerCleanupTotal,
  provisionerDriverCreatedTotal,
  provisionerGetStatusTotal,
  provisionerHealthCheckDurationSeconds,
  provisionerHealthCheckTotal,
  workspaceProvisionDurationSeconds,
  workspaceProvisionTotal,
} from '../../metrics'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import { RemoteEnvironmentDriver } from '../drivers/remote-environment-driver'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type {
  WorkspaceProvisioner,
  WorkspaceStatusResult,
} from '../types/workspace-provisioner.types'
import {
  type ExecutionTarget,
  isKubernetesExecutionTarget,
  type KubernetesExecutionTarget,
  type KubernetesPodPhase,
} from '../types/workspace-runtime.types'

/**
 * Map Kubernetes pod phase to unified EnvironmentStatus
 *
 * K8s Pod Phase → Unified Status:
 * - NotFound     → 'not_created'
 * - Pending      → 'initializing'
 * - Running      → 'ready'
 * - Succeeded    → 'stopped' (pod completed successfully)
 * - Failed       → 'error'
 * - Terminating  → 'removing'
 * - Unknown      → 'error'
 */
function mapPodPhaseToStatus(podPhase: KubernetesPodPhase | undefined): EnvironmentStatus {
  switch (podPhase) {
    case 'Pending':
      return 'initializing'
    case 'Running':
      return 'ready'
    case 'Succeeded':
      return 'stopped'
    case 'Terminating':
      return 'removing'
    case 'NotFound':
    case undefined:
      return 'not_created'
    // Failed, Unknown, and any other phases are errors
    default:
      return 'error'
  }
}

/**
 * KubernetesWorkspaceProvisioner - Handles Kubernetes workspace lifecycle
 *
 * Implements WorkspaceProvisioner for Kubernetes pod-based workspaces.
 * Manages pod creation, health checking, and cleanup.
 *
 * Responsibilities:
 * - Provision workspaces by creating Kubernetes pods
 * - Check pod health via HTTP endpoint
 * - Validate pod exists and is running
 * - Query agent status from worker pod
 * - Clean up pods
 * - Create KubernetesEnvironmentDriver instances
 */
@injectable()
export class KubernetesWorkspaceProvisioner implements WorkspaceProvisioner {
  constructor(
    @inject(KubernetesService) @optional() private readonly kubernetesService: KubernetesService,
    @inject(SettingsService) private readonly settingsService: SettingsService,
  ) {}

  /**
   * Build pod creation config with optional fields from authentication and repository settings.
   */
  private buildPodConfig(params: {
    repoConfig: RepositoryConfig
    gitCloneUrl: string
    gitBranch: string | undefined
    workerApiUrl: string
    githubToken: string | null
    anthropicApiKey: string | undefined
    claudeOAuthToken: string | null
    taskAuthToken: string
    secretName?: string
  }): PodCreationConfig {
    const {
      repoConfig,
      gitCloneUrl,
      gitBranch,
      workerApiUrl,
      githubToken,
      anthropicApiKey,
      claudeOAuthToken,
      taskAuthToken,
      secretName,
    } = params

    const podConfig: PodCreationConfig = {
      repositoryName: repoConfig.name,
      repositoryCloneUrl: gitCloneUrl,
      workerApiUrl,
    }

    if (gitBranch) {
      podConfig.repositoryBranch = gitBranch
    }
    if (githubToken) {
      podConfig.githubToken = githubToken
    }

    if (claudeOAuthToken) {
      podConfig.claudeOAuthToken = claudeOAuthToken
    } else if (anthropicApiKey) {
      podConfig.anthropicApiKey = anthropicApiKey
    } else {
      throw new Error('Cannot create worker pod: No Claude authentication configured.')
    }

    // Task-scoped JWT token for sandbox-to-backend MCP communication
    podConfig.taskAuthToken = taskAuthToken

    // Backend API URL for sandbox-to-backend communication
    podConfig.backendApiUrl = process.env['BACKEND_API_URL'] || 'http://backend:8000'

    // Per-repository worker image override
    if (repoConfig.codeExecution?.dockerImage) {
      podConfig.workerDockerImage = repoConfig.codeExecution.dockerImage
    }

    // Build repo config JSON for sandbox auto-start (same format as .moatless/config.json)
    // This enables sandbox to auto-start preview server immediately at startup
    const repoConfigFile = {
      systemPrompt: repoConfig.systemPrompt,
      codeExecution: {
        setupCommands: repoConfig.codeExecution?.setupCommands,
        dockerImage: repoConfig.codeExecution?.dockerImage,
        envVars: repoConfig.codeExecution?.envVars,
      },
      previewConfig: repoConfig.previewConfig,
    }

    // Pass repo config if there's any meaningful configuration
    const setupCommands = repoConfigFile.codeExecution?.setupCommands
    const envVars = repoConfigFile.codeExecution?.envVars
    if (
      (setupCommands && setupCommands.length > 0) ||
      (envVars && Object.keys(envVars).length > 0) ||
      repoConfigFile.codeExecution?.dockerImage ||
      repoConfigFile.previewConfig ||
      repoConfigFile.systemPrompt
    ) {
      podConfig.repoConfig = JSON.stringify(repoConfigFile)
    }

    // Pass envVars directly to pod environment
    if (repoConfig.codeExecution?.envVars) {
      podConfig.envVars = repoConfig.codeExecution.envVars
    }

    // Kubernetes Secret reference for credentials
    if (secretName) {
      podConfig.secretName = secretName
    }

    // Shared plugins PVC for Claude Code plugins (read-only, EFS/Azure Files)
    const pluginsPvcName = process.env['PLUGINS_PVC_NAME']
    if (pluginsPvcName) {
      podConfig.pluginsPvcName = pluginsPvcName
    }

    return podConfig
  }

  /**
   * Generate task-scoped JWT token for sandbox-to-backend MCP communication
   */
  private async generateTaskAuthToken(taskId: string, userId: string): Promise<string> {
    const secret = process.env['TASK_TOKEN_SECRET']
    if (!secret) {
      throw new Error('TASK_TOKEN_SECRET not configured')
    }

    const secretKey = new TextEncoder().encode(secret)
    const token = await new jose.SignJWT({ taskId, userId })
      .setProtectedHeader({ alg: 'HS256' })
      .setExpirationTime('24h')
      .setIssuedAt()
      .sign(secretKey)

    return token
  }

  /**
   * Create an initializing execution target before provisioning begins.
   * Used to save a snapshot and emit events immediately.
   */
  createInitializingTarget(_taskId: string, plan: WorkspaceProvisionPlan): ExecutionTarget {
    const k8sSettings = this.settingsService.getKubernetesSettings()
    const target: KubernetesExecutionTarget = {
      mode: 'kubernetes',
      podNamespace: k8sSettings.namespace,
      status: 'initializing',
      repositoryCloneUrl: plan.gitCloneUrl,
      workerPort: 8001,
    }
    if (plan.gitBranch) {
      target.repositoryBranch = plan.gitBranch
    }
    return target
  }

  /**
   * Provision a Kubernetes workspace by creating a pod
   *
   * Steps:
   * 1. Get repository configuration and git clone URL
   * 2. Resolve authentication (GitHub token, Claude token, worker auth)
   * 3. Create pod with environment configuration
   * 4. Wait for pod to become ready
   *
   * @returns KubernetesExecutionTarget with pod info
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Pod provisioning requires many conditional steps
  async provision(taskId: string, plan: WorkspaceProvisionPlan): Promise<ExecutionTarget> {
    const startTime = Date.now()
    const {
      repoConfig,
      gitCloneUrl,
      gitBranch,
      gitProviderToken,
      claudeOAuthToken,
      anthropicApiKey,
    } = plan
    logger.info(
      { taskId, hasKubernetesService: !!this.kubernetesService },
      'KubernetesWorkspaceProvisioner: provision() called',
    )

    if (!this.kubernetesService) {
      logger.error({ taskId }, 'KubernetesWorkspaceProvisioner: Kubernetes service not available')
      throw new Error('Kubernetes service not available')
    }

    const k8sSettings = this.settingsService.getKubernetesSettings()
    logger.info(
      { taskId, namespace: k8sSettings.namespace },
      'KubernetesWorkspaceProvisioner: Got k8s settings',
    )

    // 1. Reuse existing pod if healthy or pending
    logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: Checking for existing pod')
    const existingTarget = await this.checkExistingPod(taskId, gitCloneUrl, gitBranch)
    if (existingTarget) {
      logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: Returning existing pod')
      return existingTarget
    }
    logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: No existing pod found')

    logger.info(
      { taskId, repositoryName: repoConfig.name, gitCloneUrl },
      'KubernetesWorkspaceProvisioner: Creating Kubernetes pod',
    )

    // 2. Generate task-scoped auth token (K8s-specific: sandbox-to-backend MCP JWT)
    const { SessionContext } = await import('../../auth/services/session-context')
    const session = SessionContext.getCurrentSession()
    if (!session) {
      throw new Error('Cannot create worker pod: No authenticated user session.')
    }
    const taskAuthToken = await this.generateTaskAuthToken(taskId, session.userId)

    // 3. Create PVC for workspace persistence (if enabled)
    let pvcName: string | null = null
    try {
      // Try to find existing PVC first (for pod restart case)
      pvcName = await this.kubernetesService.findTaskPvc(taskId)
      if (pvcName) {
        logger.info(
          { taskId, pvcName },
          'KubernetesWorkspaceProvisioner: Found existing PVC, reusing',
        )
      } else {
        pvcName = await this.kubernetesService.createTaskPvc(taskId)
        if (pvcName) {
          logger.info(
            { taskId, pvcName },
            'KubernetesWorkspaceProvisioner: Created new PVC for workspace',
          )
        }
      }
    } catch (error) {
      logger.warn(
        { taskId, error: (error as Error).message },
        'KubernetesWorkspaceProvisioner: Failed to create PVC, proceeding without persistence',
      )
    }

    // 4. Create task secret with credentials
    let secretName: string | undefined
    try {
      // Try to reuse existing secret first
      secretName = (await this.kubernetesService.findTaskSecret(taskId)) ?? undefined
      if (!secretName) {
        const secretCredentials: import('../../../services/kubernetes.service').TaskCredentials = {
          taskAuthToken,
          backendApiUrl: process.env['BACKEND_API_URL'] || 'http://backend:8000',
          githubToken: gitProviderToken,
        }
        if (anthropicApiKey) {
          secretCredentials.anthropicApiKey = anthropicApiKey
        }
        if (claudeOAuthToken) {
          secretCredentials.claudeOAuthToken = claudeOAuthToken
        }

        secretName = await this.kubernetesService.createTaskSecret(taskId, secretCredentials)
        logger.info(
          { taskId, secretName },
          'KubernetesWorkspaceProvisioner: Created task credentials secret',
        )
      } else {
        logger.info(
          { taskId, secretName },
          'KubernetesWorkspaceProvisioner: Found existing secret, reusing',
        )
      }
    } catch (error) {
      logger.warn(
        { taskId, error: (error as Error).message },
        'KubernetesWorkspaceProvisioner: Failed to create task secret, using inline env vars',
      )
    }

    // 5. Build pod config
    const podConfigParams: Parameters<typeof this.buildPodConfig>[0] = {
      repoConfig,
      gitCloneUrl,
      gitBranch,
      workerApiUrl: k8sSettings.workerApiUrl || '',
      githubToken: gitProviderToken,
      anthropicApiKey: anthropicApiKey,
      claudeOAuthToken: claudeOAuthToken,
      taskAuthToken,
    }
    if (secretName) {
      podConfigParams.secretName = secretName
    }
    const podConfig = this.buildPodConfig(podConfigParams)

    // Add PVC name to pod config if created
    if (pvcName) {
      podConfig.pvcName = pvcName
    }

    // 6. Create pod
    let podName: string | undefined
    let podNamespace: string | undefined

    try {
      const createResult = await this.kubernetesService.createPod(taskId, podConfig)
      podName = createResult.podName
      podNamespace = createResult.podNamespace

      kubernetesPodTotal.labels({ status: 'created' }).inc()

      // 7. Wait for pod to be ready
      logger.info(
        { taskId, podName },
        'KubernetesWorkspaceProvisioner: Waiting for pod to be ready',
      )
      const podStatus = await this.kubernetesService.waitForPodReady(taskId)

      // 8. Create execution target
      const target: KubernetesExecutionTarget = {
        mode: 'kubernetes',
        podName,
        podNamespace,
        podPhase: podStatus.phase,
        podIp: podStatus.podIp,
        status: mapPodPhaseToStatus(podStatus.phase), // Unified status
        repositoryCloneUrl: gitCloneUrl,
        workerPort: 8001,
        previewHostPort: 3000,
      }

      if (gitBranch) {
        target.repositoryBranch = gitBranch
      }

      // Track metrics
      const durationSeconds = (Date.now() - startTime) / 1000
      workspaceProvisionTotal.labels({ mode: 'kubernetes', status: 'success' }).inc()
      workspaceProvisionDurationSeconds.labels({ mode: 'kubernetes' }).observe(durationSeconds)
      kubernetesPodTotal.labels({ status: 'ready' }).inc()
      kubernetesPodReadinessDurationSeconds.observe(durationSeconds)

      logger.info({ taskId, podName }, 'KubernetesWorkspaceProvisioner: Workspace provisioned')

      return target
    } catch (error) {
      // Track error metrics
      workspaceProvisionTotal.labels({ mode: 'kubernetes', status: 'error' }).inc()
      kubernetesPodTotal.labels({ status: 'failed' }).inc()

      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { taskId, error: errorMessage, podName },
        'KubernetesWorkspaceProvisioner: Pod provisioning failed',
      )

      throw error
    }
  }

  /**
   * Check if pod is healthy via HTTP health endpoint
   *
   * Used by driver cache to detect stale drivers pointing to dead pods.
   */
  async isHealthy(taskId: string, target: ExecutionTarget): Promise<boolean> {
    const startTime = Date.now()

    if (!isKubernetesExecutionTarget(target)) {
      logger.warn('KubernetesWorkspaceProvisioner: isHealthy called with non-kubernetes target')
      provisionerHealthCheckTotal.labels({ mode: 'kubernetes', result: 'error' }).inc()
      return false
    }

    if (!this.kubernetesService) {
      provisionerHealthCheckTotal.labels({ mode: 'kubernetes', result: 'error' }).inc()
      return false
    }

    // Get pod IP on-demand (not stored in target)
    const podIp = await this.kubernetesService.getPodIp(taskId)
    if (!podIp) {
      const durationSeconds = (Date.now() - startTime) / 1000
      provisionerHealthCheckDurationSeconds.labels({ mode: 'kubernetes' }).observe(durationSeconds)
      provisionerHealthCheckTotal.labels({ mode: 'kubernetes', result: 'unhealthy' }).inc()
      logger.debug(
        { podName: target.podName },
        'KubernetesWorkspaceProvisioner: Pod has no IP, not healthy',
      )
      return false
    }

    const isHealthy = await this.kubernetesService.checkPodHealth(podIp)

    // Track metrics
    const durationSeconds = (Date.now() - startTime) / 1000
    provisionerHealthCheckDurationSeconds.labels({ mode: 'kubernetes' }).observe(durationSeconds)
    provisionerHealthCheckTotal
      .labels({ mode: 'kubernetes', result: isHealthy ? 'healthy' : 'unhealthy' })
      .inc()

    if (!isHealthy) {
      logger.debug(
        { podName: target.podName, podIp },
        'KubernetesWorkspaceProvisioner: Pod health check failed',
      )
    }

    return isHealthy
  }

  /**
   * Get pod status for logging and decision making
   */
  async getStatus(taskId: string, target: ExecutionTarget): Promise<WorkspaceStatusResult> {
    if (!isKubernetesExecutionTarget(target)) {
      provisionerGetStatusTotal.labels({ mode: 'kubernetes', status: 'error' }).inc()
      return { status: 'error', lastError: 'Non-kubernetes target' }
    }

    if (!this.kubernetesService) {
      provisionerGetStatusTotal.labels({ mode: 'kubernetes', status: 'error' }).inc()
      return { status: 'error', lastError: 'Kubernetes service not available' }
    }

    const podStatus = await this.kubernetesService.getPodStatus(taskId)
    if (!podStatus) {
      provisionerGetStatusTotal.labels({ mode: 'kubernetes', status: 'not_created' }).inc()
      return { status: 'not_created', podName: target.podName }
    }

    // Map K8s pod phases to unified status
    const status = mapPodPhaseToStatus(podStatus.phase)
    provisionerGetStatusTotal.labels({ mode: 'kubernetes', status }).inc()

    // Extract error message from pod conditions if pod failed
    let lastError = target.lastError
    if (podStatus.phase === 'Failed' && podStatus.conditions) {
      const failedCondition = podStatus.conditions.find((c) => c.status === 'False' && c.message)
      if (failedCondition?.message) {
        lastError = failedCondition.message
      }
    }

    return {
      status,
      podName: target.podName,
      lastError,
    }
  }

  /**
   * Check if agent is running by querying worker pod's /agent-status endpoint
   */
  async isAgentRunning(taskId: string, target: ExecutionTarget): Promise<boolean> {
    if (!isKubernetesExecutionTarget(target)) {
      return false
    }

    if (!this.kubernetesService) {
      return false
    }

    // Pod name not known yet (initializing state)
    if (!target.podName) {
      return false
    }

    const podIp = await this.kubernetesService.getPodIp(target.podName)
    if (!podIp) {
      return false
    }

    try {
      const workerUrl = `http://${podIp}:${target.workerPort}/agent-status`
      const response = await fetch(workerUrl, {
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        return false
      }

      const data = (await response.json()) as { state: string }
      return data.state === 'running'
    } catch (error) {
      logger.debug(
        { taskId, podIp, err: error },
        'KubernetesWorkspaceProvisioner: Worker unreachable',
      )
      return false
    }
  }

  /**
   * Clean up by deleting the pod, PVC, and task secret
   */
  async cleanup(
    taskId: string,
    target: ExecutionTarget,
    _plan?: WorkspaceProvisionPlan,
  ): Promise<void> {
    if (!isKubernetesExecutionTarget(target)) {
      logger.warn('KubernetesWorkspaceProvisioner: cleanup called with non-kubernetes target')
      return
    }

    if (!this.kubernetesService) {
      return
    }

    logger.info(
      { taskId, podName: target.podName },
      'KubernetesWorkspaceProvisioner: Deleting pod, PVC, and secret',
    )

    // 1. Delete the pod first
    try {
      await this.kubernetesService.deletePod(taskId)
      provisionerCleanupTotal.labels({ mode: 'kubernetes', status: 'success' }).inc()
      logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: Pod deleted')
    } catch (error) {
      provisionerCleanupTotal.labels({ mode: 'kubernetes', status: 'error' }).inc()
      logger.warn(
        { taskId, error: (error as Error).message },
        'KubernetesWorkspaceProvisioner: Failed to delete pod (may already be deleted)',
      )
    }

    // 2. Delete the PVC (this removes all persisted workspace data)
    try {
      await this.kubernetesService.deleteTaskPvc(taskId)
      logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: PVC deleted')
    } catch (error) {
      logger.warn(
        { taskId, error: (error as Error).message },
        'KubernetesWorkspaceProvisioner: Failed to delete PVC (may already be deleted)',
      )
    }

    // 3. Delete the task secret
    try {
      await this.kubernetesService.deleteTaskSecret(taskId)
      logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: Task secret deleted')
    } catch (error) {
      logger.warn(
        { taskId, error: (error as Error).message },
        'KubernetesWorkspaceProvisioner: Failed to delete task secret (may already be deleted)',
      )
    }
  }

  /**
   * Clear error state by deleting only the failed pod
   * Preserves PVC and secret for reuse on retry
   */
  async clearError(taskId: string, target: ExecutionTarget): Promise<void> {
    if (!isKubernetesExecutionTarget(target)) {
      logger.warn('KubernetesWorkspaceProvisioner: clearError called with non-kubernetes target')
      return
    }

    if (!this.kubernetesService) {
      return
    }

    logger.info(
      { taskId, podName: target.podName },
      'KubernetesWorkspaceProvisioner: Deleting pod only (preserving PVC and secret for retry)',
    )

    try {
      await this.kubernetesService.deletePod(taskId)
      logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: Pod deleted')
    } catch (error) {
      logger.warn(
        { taskId, error: (error as Error).message },
        'KubernetesWorkspaceProvisioner: Failed to delete pod (may already be deleted)',
      )
    }
  }

  /**
   * Create a RemoteEnvironmentDriver for the Kubernetes workspace
   *
   * The driver uses HTTP to communicate with the worker pod.
   * URL resolution happens on-demand via getPodIp() to handle pod restarts.
   */
  createDriver(taskId: string, target: ExecutionTarget): BaseEnvironmentDriver {
    if (!isKubernetesExecutionTarget(target)) {
      throw new Error(
        'KubernetesWorkspaceProvisioner: Cannot create driver for non-kubernetes target',
      )
    }

    if (!this.kubernetesService) {
      throw new Error('Kubernetes service not available')
    }

    const kubernetesService = this.kubernetesService

    provisionerDriverCreatedTotal.labels({ mode: 'kubernetes' }).inc()
    return new RemoteEnvironmentDriver({
      taskId,
      getWorkerUrl: async () => {
        const podIp = await kubernetesService.getPodIp(taskId)
        if (!podIp) {
          throw new Error(`Pod IP not available for task: ${taskId}`)
        }
        return `http://${podIp}:${target.workerPort}`
      },
      getPreviewUrl: async () => {
        const podIp = await kubernetesService.getPodIp(taskId)
        if (!podIp) {
          return undefined
        }
        return `http://${podIp}:${target.previewHostPort}`
      },
    })
  }

  /**
   * Check if an existing pod can be reused
   * Returns the execution target if pod is healthy, null otherwise
   */
  async checkExistingPod(
    taskId: string,
    repositoryCloneUrl: string,
    repositoryBranch?: string,
  ): Promise<ExecutionTarget | null> {
    if (!this.kubernetesService) {
      return null
    }

    const existingPod = await this.kubernetesService.findExistingPod(taskId)
    if (!existingPod) {
      return null
    }

    if (existingPod.phase === 'Running' && existingPod.podIp) {
      const isHealthy = await this.kubernetesService.checkPodHealth(existingPod.podIp)
      if (isHealthy) {
        kubernetesPodTotal.labels({ status: 'reused' }).inc()
        logger.debug(
          { taskId, podName: existingPod.podName },
          'KubernetesWorkspaceProvisioner: Reusing healthy existing pod',
        )

        const k8sSettings = this.settingsService.getKubernetesSettings()
        const target: KubernetesExecutionTarget = {
          mode: 'kubernetes',
          podName: existingPod.podName,
          podNamespace: k8sSettings.namespace,
          podPhase: existingPod.phase,
          status: 'ready', // Unified status
          repositoryCloneUrl,
          workerPort: 8001,
          previewHostPort: 3000,
        }
        if (repositoryBranch) {
          target.repositoryBranch = repositoryBranch
        }
        return target
      }
    }

    if (existingPod.phase === 'Pending') {
      // Pod is starting - wait for it
      logger.info(
        { taskId, podName: existingPod.podName },
        'KubernetesWorkspaceProvisioner: Waiting for pending pod',
      )

      try {
        const podStatus = await this.kubernetesService.waitForPodReady(taskId)
        const k8sSettings = this.settingsService.getKubernetesSettings()

        const target: KubernetesExecutionTarget = {
          mode: 'kubernetes',
          podName: existingPod.podName,
          podNamespace: k8sSettings.namespace,
          podPhase: podStatus.phase,
          status: mapPodPhaseToStatus(podStatus.phase), // Unified status
          repositoryCloneUrl,
          workerPort: 8001,
          previewHostPort: 3000,
        }
        if (repositoryBranch) {
          target.repositoryBranch = repositoryBranch
        }
        return target
      } catch (error) {
        if (error instanceof WorkspaceError) {
          throw error // Permanent failure
        }
        // Transient error - delete pod and let caller create new one
        await this.kubernetesService.deletePod(taskId)
        return null
      }
    }

    // Failed/Succeeded pods should be deleted
    if (existingPod.phase === 'Failed' || existingPod.phase === 'Succeeded') {
      await this.kubernetesService.deletePod(taskId)
    }

    return null
  }
}
