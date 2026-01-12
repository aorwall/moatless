import type { RepositoryConfig } from '@moatless/types'
import { WorkspaceError } from '@moatless/types'
import { inject, injectable, optional } from 'inversify'
import { KubernetesService, type PodCreationConfig } from '../../../services/kubernetes.service'
import { SettingsService } from '../../../services/settings.service'
import { logger } from '../../../utils/logger'
import { GitProviderTokenService } from '../../auth/services/git-provider-token.service'
import { WorkerTokenService } from '../../auth/services/worker-token.service'
import { ClaudeTokenService } from '../../claude/services/claude-token.service'
import { EventService } from '../../events/services/event.service'
import {
  kubernetesPodReadinessDurationSeconds,
  kubernetesPodTotal,
  provisionerCleanupTotal,
  provisionerDriverCreatedTotal,
  provisionerHealthCheckDurationSeconds,
  provisionerHealthCheckTotal,
  provisionerValidateTotal,
  workspaceProvisionDurationSeconds,
  workspaceProvisionTotal,
} from '../../metrics'
import { RepositoryService } from '../../repositories/index'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import { RemoteEnvironmentDriver } from '../drivers/remote-environment-driver'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { WorkspaceProvisioner } from '../types/workspace-provisioner.types'
import {
  type ExecutionTarget,
  isKubernetesExecutionTarget,
  type KubernetesExecutionTarget,
} from '../types/workspace-runtime.types'

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
    @inject(GitProviderTokenService)
    private readonly gitProviderTokenService: GitProviderTokenService,
    @inject(ClaudeTokenService) private readonly claudeTokenService: ClaudeTokenService,
    @inject(WorkerTokenService) private readonly workerTokenService: WorkerTokenService,
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(EventService) private readonly eventService: EventService,
  ) {}

  /**
   * Build pod creation config with optional fields from authentication and repository settings.
   */
  private buildPodConfig(params: {
    repoConfig: RepositoryConfig
    gitCloneUrl: string
    gitBranch: string | undefined
    workerApiUrl: string
    workerAuthToken: string
    githubToken: string | null
    anthropicApiKey: string | undefined
    claudeOAuthToken: string | null
  }): PodCreationConfig {
    const {
      repoConfig,
      gitCloneUrl,
      gitBranch,
      workerApiUrl,
      workerAuthToken,
      githubToken,
      anthropicApiKey,
      claudeOAuthToken,
    } = params

    const podConfig: PodCreationConfig = {
      repositoryName: repoConfig.name,
      repositoryCloneUrl: gitCloneUrl,
      workerApiUrl,
      workerAuthToken,
    }

    if (gitBranch) {
      podConfig.repositoryBranch = gitBranch
    }
    if (githubToken) {
      podConfig.githubToken = githubToken
    }
    if (anthropicApiKey && anthropicApiKey !== 'placeholder' && !claudeOAuthToken) {
      podConfig.anthropicApiKey = anthropicApiKey
    }
    if (claudeOAuthToken) {
      podConfig.claudeOAuthToken = claudeOAuthToken
    }

    const databaseUrl = process.env['DATABASE_URL']
    if (databaseUrl) {
      podConfig.databaseUrl = databaseUrl
    }

    // Per-repository worker image override
    if (repoConfig.codeExecution?.dockerImage) {
      podConfig.workerDockerImage = repoConfig.codeExecution.dockerImage
    }

    return podConfig
  }

  /**
   * Provide a Pending execution target so clients can see initializing state.
   */
  async getInitializingTarget(
    taskId: string,
    repositoryId: string,
    _plan: WorkspaceProvisionPlan,
  ): Promise<ExecutionTarget | null> {
    if (!this.kubernetesService) {
      return null
    }

    const repoConfig = await this.repoService.getRepositoryById(repositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repositoryId}`)
    }
    const gitCloneUrl = repoConfig.remote.url
    const k8sSettings = this.settingsService.getKubernetesSettings()

    return {
      mode: 'kubernetes',
      podName: `moatless-${taskId.slice(0, 8)}`,
      podNamespace: k8sSettings.namespace,
      podPhase: 'Pending',
      repositoryCloneUrl: gitCloneUrl,
      workerPort: 8001,
    }
  }

  /**
   * Resolve authentication tokens required for pod creation
   */
  private async resolveAuthentication(taskId: string, gitCloneUrl: string) {
    const githubToken = await this.gitProviderTokenService.getTokenForGitOperations(gitCloneUrl)
    const anthropicApiKey = process.env['ANTHROPIC_API_KEY']

    let claudeOAuthToken = await this.claudeTokenService.getTokenForAgent()
    if (!claudeOAuthToken) {
      claudeOAuthToken = process.env['CLAUDE_CODE_OAUTH_TOKEN'] ?? null
    }

    const hasValidAnthropicKey = anthropicApiKey && anthropicApiKey !== 'placeholder'
    if (!(claudeOAuthToken || hasValidAnthropicKey)) {
      throw new Error(
        'Cannot create worker pod: No Claude authentication configured. ' +
          'Please connect a Claude account (Settings → Claude) or set ANTHROPIC_API_KEY.',
      )
    }

    const { SessionContext } = await import('../../auth/services/session-context')
    const session = SessionContext.getCurrentSession()
    if (!session) {
      throw new Error('Cannot create worker pod: No authenticated user session.')
    }

    const workerAuthToken = await this.workerTokenService.generateToken({
      userId: session.userId,
      taskId,
      userLogin: session.userLogin,
      userEmail: session.userEmail ?? undefined,
      groupId: session.groupId,
    })

    return { githubToken, anthropicApiKey, claudeOAuthToken, workerAuthToken }
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
  async provision(
    taskId: string,
    repositoryId: string,
    _plan: WorkspaceProvisionPlan,
  ): Promise<ExecutionTarget> {
    const startTime = Date.now()
    logger.info(
      { taskId, repositoryId, hasKubernetesService: !!this.kubernetesService },
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

    // 1. Resolve repository and clone URL
    logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: Resolving repository config')
    const repoConfig = await this.repoService.getRepositoryById(repositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repositoryId}`)
    }
    logger.info({ taskId }, 'KubernetesWorkspaceProvisioner: Resolving clone URL')
    const gitCloneUrl = repoConfig.remote.url
    const gitBranch: string | undefined = undefined

    // 2. Reuse existing pod if healthy or pending
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

    // 3. Get authentication tokens
    const { githubToken, anthropicApiKey, claudeOAuthToken, workerAuthToken } =
      await this.resolveAuthentication(taskId, gitCloneUrl)

    // 4. Create PVC for workspace persistence (if enabled)
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

    // 5. Build pod config
    const podConfig = this.buildPodConfig({
      repoConfig,
      gitCloneUrl,
      gitBranch,
      workerApiUrl: k8sSettings.workerApiUrl || '',
      workerAuthToken,
      githubToken,
      anthropicApiKey,
      claudeOAuthToken,
    })

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

      // 6. Wait for pod to be ready
      logger.info(
        { taskId, podName },
        'KubernetesWorkspaceProvisioner: Waiting for pod to be ready',
      )
      const podStatus = await this.kubernetesService.waitForPodReady(taskId)

      // 7. Create execution target
      const target: KubernetesExecutionTarget = {
        mode: 'kubernetes',
        podName,
        podNamespace,
        podPhase: podStatus.phase,
        repositoryCloneUrl: gitCloneUrl,
        workerPort: 8001,
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
   * Validate pod exists and is in a provisioned phase.
   */
  async validate(taskId: string, target: ExecutionTarget): Promise<boolean> {
    if (!isKubernetesExecutionTarget(target)) {
      logger.warn('KubernetesWorkspaceProvisioner: validate called with non-kubernetes target')
      provisionerValidateTotal.labels({ mode: 'kubernetes', result: 'error' }).inc()
      return false
    }

    if (!this.kubernetesService) {
      provisionerValidateTotal.labels({ mode: 'kubernetes', result: 'error' }).inc()
      return false
    }

    const podStatus = await this.kubernetesService.getPodStatus(taskId)
    if (!podStatus) {
      provisionerValidateTotal.labels({ mode: 'kubernetes', result: 'invalid' }).inc()
      return false
    }

    const isValid = podStatus.phase === 'Running'
    provisionerValidateTotal
      .labels({ mode: 'kubernetes', result: isValid ? 'valid' : 'invalid' })
      .inc()

    return isValid
  }

  /**
   * Refresh execution target state by checking live pod status.
   */
  async refreshExecutionTarget(taskId: string, target: ExecutionTarget): Promise<ExecutionTarget> {
    if (!(isKubernetesExecutionTarget(target) && this.kubernetesService)) {
      return target
    }

    const existingPod = await this.kubernetesService.findExistingPod(taskId)
    if (!existingPod) {
      return {
        ...target,
        podPhase: 'NotFound',
        lastHealthCheck: new Date(),
      }
    }

    return {
      ...target,
      podPhase: existingPod.phase,
      lastHealthCheck: new Date(),
      lastError: existingPod.phase === 'Running' ? undefined : target.lastError,
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
   * Clean up by deleting the pod and PVC
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
      'KubernetesWorkspaceProvisioner: Deleting pod and PVC',
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
  }

  /**
   * Clear error state by deleting the failed pod
   */
  async clearError(taskId: string, target: ExecutionTarget): Promise<void> {
    // Same as cleanup - delete the pod
    await this.cleanup(taskId, target)
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
      eventService: this.eventService,
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
          repositoryCloneUrl,
          workerPort: 8001,
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
          repositoryCloneUrl,
          workerPort: 8001,
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
