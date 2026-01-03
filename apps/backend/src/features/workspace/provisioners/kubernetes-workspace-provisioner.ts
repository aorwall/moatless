import { type RepositoryConfig, WorkspaceError } from '@moatless/types'
import { inject, injectable, optional } from 'inversify'
import { KubernetesService } from '../../../services/kubernetes.service'
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
} from '../../../services/metrics'
import { SettingsService } from '../../../services/settings.service'
import { logger } from '../../../utils/logger'
import { GitProviderTokenService } from '../../auth/services/git-provider-token.service'
import { WorkerTokenService } from '../../auth/services/worker-token.service'
import { ClaudeTokenService } from '../../claude/services/claude-token.service'
import { EventService } from '../../events/services/event.service'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import { KubernetesEnvironmentDriver } from '../drivers/kubernetes-environment-driver'
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
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(EventService) private readonly eventService: EventService,
  ) {}

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
    logger.debug({ taskId, repositoryId }, 'KubernetesWorkspaceProvisioner: Provisioning workspace')

    if (!this.kubernetesService) {
      throw new Error('Kubernetes service not available')
    }

    const k8sSettings = this.settingsService.getKubernetesSettings()

    // 1. Get repository config
    const repoConfig = await this.repoService.getRepositoryById(repositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repositoryId}`)
    }

    // 2. Get git clone URL
    const gitCloneUrl = await this.resolveGitCloneUrl(taskId, repoConfig)
    const gitBranch: string | undefined = undefined

    logger.info(
      { taskId, repositoryName: repoConfig.name, gitCloneUrl },
      'KubernetesWorkspaceProvisioner: Creating Kubernetes pod',
    )

    // 3. Get authentication tokens
    const githubToken = await this.gitProviderTokenService.getTokenForGitOperations(gitCloneUrl)
    const anthropicApiKey = process.env['ANTHROPIC_API_KEY']

    // Get Claude OAuth token
    const { SessionContext } = await import('../../auth/services/session-context')

    let claudeOAuthToken = await this.claudeTokenService.getTokenForAgent()
    if (!claudeOAuthToken) {
      claudeOAuthToken = process.env['CLAUDE_CODE_OAUTH_TOKEN'] ?? null
    }

    // Validate authentication
    const hasValidAnthropicKey = anthropicApiKey && anthropicApiKey !== 'placeholder'
    if (!(claudeOAuthToken || hasValidAnthropicKey)) {
      throw new Error(
        'Cannot create worker pod: No Claude authentication configured. ' +
          'Please connect a Claude account (Settings → Claude) or set ANTHROPIC_API_KEY.',
      )
    }

    // Get session and generate worker auth token
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

    // 4. Build pod config
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

    // 5. Create pod
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
  async isHealthy(target: ExecutionTarget): Promise<boolean> {
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
    const podIp = await this.kubernetesService.getPodIp(target.podName)
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
   * Validate pod exists and is in Running phase
   */
  async validate(target: ExecutionTarget): Promise<boolean> {
    if (!isKubernetesExecutionTarget(target)) {
      logger.warn('KubernetesWorkspaceProvisioner: validate called with non-kubernetes target')
      provisionerValidateTotal.labels({ mode: 'kubernetes', result: 'error' }).inc()
      return false
    }

    if (!this.kubernetesService) {
      provisionerValidateTotal.labels({ mode: 'kubernetes', result: 'error' }).inc()
      return false
    }

    const podStatus = await this.kubernetesService.getPodStatus(target.podName)
    if (!podStatus) {
      provisionerValidateTotal.labels({ mode: 'kubernetes', result: 'invalid' }).inc()
      return false
    }

    const isRunning = podStatus.phase === 'Running'
    provisionerValidateTotal
      .labels({ mode: 'kubernetes', result: isRunning ? 'valid' : 'invalid' })
      .inc()

    return isRunning
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
   * Clean up by deleting the pod
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

    logger.info({ taskId, podName: target.podName }, 'KubernetesWorkspaceProvisioner: Deleting pod')

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
  }

  /**
   * Clear error state by deleting the failed pod
   */
  async clearError(taskId: string, target: ExecutionTarget): Promise<void> {
    // Same as cleanup - delete the pod
    await this.cleanup(taskId, target)
  }

  /**
   * Create a KubernetesEnvironmentDriver for the workspace
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

    provisionerDriverCreatedTotal.labels({ mode: 'kubernetes' }).inc()
    return new KubernetesEnvironmentDriver(
      taskId,
      target,
      this.eventService,
      this.kubernetesService,
    )
  }

  /**
   * Resolve git clone URL based on repository type
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
            'KubernetesWorkspaceProvisioner: Using template remote URL',
          )
          return templateRepo.remote.url
        }
        throw new Error(`Template repository not found: ${task.templateId}`)
      }
      throw new Error('Local repositories without templates are not supported in Kubernetes mode')
    }

    throw new Error('Template repositories cannot be used directly for tasks')
  }

  /**
   * Check if an existing pod can be reused
   * Returns the execution target if pod is healthy, null otherwise
   */
  async checkExistingPod(taskId: string): Promise<ExecutionTarget | null> {
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
          repositoryCloneUrl: '', // Will be resolved if needed
          workerPort: 8001,
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
          repositoryCloneUrl: '', // Will be resolved if needed
          workerPort: 8001,
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
