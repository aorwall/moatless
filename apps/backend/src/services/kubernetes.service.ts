import * as fs from 'node:fs'
import * as k8s from '@kubernetes/client-node'
import { WorkspaceError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import yaml from 'js-yaml'
import { logger } from '../utils/logger'
import { SettingsService } from './settings.service'

/**
 * Pod phase type (matching Kubernetes API)
 */
export type PodPhase = 'Pending' | 'Running' | 'Succeeded' | 'Failed' | 'Unknown'

/**
 * Pod creation configuration
 */
export interface PodCreationConfig {
  repositoryName: string
  repositoryCloneUrl: string
  repositoryBranch?: string
  workerApiUrl: string
  workerAuthToken: string // JWT token for user authentication (required)
  githubToken?: string
  anthropicApiKey?: string
  claudeOAuthToken?: string
}

/**
 * Pod status information
 */
export interface PodStatusInfo {
  phase: PodPhase
  podIp?: string
  conditions?: Array<{
    type: string
    status: string
    reason?: string
    message?: string
  }>
  containerStatuses?: Array<{
    name: string
    ready: boolean
    restartCount: number
    state?: string
  }>
}

/**
 * Result of pod creation
 */
export interface PodCreationResult {
  podName: string
  podNamespace: string
}

/**
 * KubernetesService - Manages Kubernetes pod lifecycle for task execution
 *
 * This service handles:
 * - Pod creation with proper configuration
 * - Pod deletion and cleanup
 * - Pod status monitoring
 * - Health checking of worker pods
 */
@injectable()
export class KubernetesService {
  private coreApi: k8s.CoreV1Api | null = null
  private kubeConfig: k8s.KubeConfig | null = null
  private initialized = false
  private initError: string | null = null

  constructor(@inject(SettingsService) private readonly settingsService: SettingsService) {}

  /**
   * Initialize Kubernetes client
   * Lazy initialization to avoid errors when k8s is not configured
   */
  private ensureInitialized(): void {
    if (this.initialized) {
      if (this.initError) {
        throw new Error(`Kubernetes not available: ${this.initError}`)
      }
      return
    }

    try {
      this.kubeConfig = new k8s.KubeConfig()

      const k8sSettings = this.settingsService.getKubernetesSettings()

      if (k8sSettings.kubeconfig) {
        // Load from specified kubeconfig file
        this.kubeConfig.loadFromFile(k8sSettings.kubeconfig)
      } else if (process.env['KUBERNETES_SERVICE_HOST']) {
        // Running inside a pod - use in-cluster config
        this.kubeConfig.loadFromCluster()
      } else {
        // Load from default kubeconfig location
        this.kubeConfig.loadFromDefault()
      }

      this.coreApi = this.kubeConfig.makeApiClient(k8s.CoreV1Api)
      this.initialized = true

      logger.info('KubernetesService: Successfully initialized Kubernetes client')
    } catch (error) {
      this.initError = error instanceof Error ? error.message : String(error)
      this.initialized = true // Mark as initialized to avoid retrying
      logger.warn(
        { error: this.initError },
        'KubernetesService: Failed to initialize Kubernetes client',
      )
      throw new Error(`Kubernetes not available: ${this.initError}`)
    }
  }

  /**
   * Check if Kubernetes is available and configured
   */
  async isKubernetesAvailable(): Promise<boolean> {
    try {
      this.ensureInitialized()
      // Try to list namespaces as a health check
      await this.coreApi?.listNamespace({ limit: 1 })
      return true
    } catch {
      return false
    }
  }

  /**
   * Generate a unique pod name for a task
   */
  generatePodName(repositoryName: string, taskId: string): string {
    // Kubernetes pod names must be lowercase, alphanumeric, and max 63 chars
    const sanitizedRepo = repositoryName
      .toLowerCase()
      .replace(/[^a-z0-9-]/g, '-')
      .substring(0, 20)
    const sanitizedTask = taskId
      .toLowerCase()
      .replace(/[^a-z0-9-]/g, '-')
      .substring(0, 20)
    const timestamp = Date.now().toString(36).substring(-6)

    return `moatless-${sanitizedRepo}-${sanitizedTask}-${timestamp}`.substring(0, 63)
  }

  /**
   * Create a new pod for task execution
   */
  async createPod(taskId: string, config: PodCreationConfig): Promise<PodCreationResult> {
    this.ensureInitialized()

    const k8sSettings = this.settingsService.getKubernetesSettings()
    const namespace = k8sSettings.namespace
    const podName = this.generatePodName(config.repositoryName, taskId)

    logger.info(
      {
        taskId,
        podName,
        namespace,
        repositoryCloneUrl: config.repositoryCloneUrl,
        usingTemplate: !!k8sSettings.workerPodTemplate,
      },
      'KubernetesService: Creating pod for task',
    )

    // Build pod spec from template or from scratch
    const podSpec = k8sSettings.workerPodTemplate
      ? this.buildPodFromTemplate(k8sSettings.workerPodTemplate, taskId, podName, namespace, config)
      : this.buildPodFromSettings(taskId, podName, namespace, config, k8sSettings)

    try {
      await this.coreApi?.createNamespacedPod({ namespace, body: podSpec })

      logger.info({ taskId, podName, namespace }, 'KubernetesService: Pod created successfully')

      return { podName, podNamespace: namespace }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, taskId, podName, namespace },
        'KubernetesService: Failed to create pod',
      )
      throw new Error(`Failed to create pod: ${message}`)
    }
  }

  /**
   * Build pod spec from a YAML template file
   */
  private buildPodFromTemplate(
    templatePath: string,
    taskId: string,
    podName: string,
    namespace: string,
    config: PodCreationConfig,
  ): k8s.V1Pod {
    // Read and parse template
    const templateContent = fs.readFileSync(templatePath, 'utf8')
    const template = yaml.load(templateContent) as k8s.V1Pod

    // Build task-specific environment variables
    const taskEnvVars: k8s.V1EnvVar[] = [
      { name: 'WORKER_MODE', value: 'true' },
      { name: 'WORKER_TASK_ID', value: taskId },
      { name: 'WORKER_API_URL', value: config.workerApiUrl },
      { name: 'MCP_SERVER_URL', value: config.workerApiUrl }, // MCP endpoint on main API
      { name: 'WORKER_AUTH_TOKEN', value: config.workerAuthToken },
      { name: 'GIT_CLONE_URL', value: config.repositoryCloneUrl },
      ...(config.repositoryBranch ? [{ name: 'GIT_BRANCH', value: config.repositoryBranch }] : []),
      ...(config.githubToken ? [{ name: 'GITHUB_TOKEN', value: config.githubToken }] : []),
      ...(config.anthropicApiKey
        ? [{ name: 'ANTHROPIC_API_KEY', value: config.anthropicApiKey }]
        : []),
      ...(config.claudeOAuthToken
        ? [{ name: 'CLAUDE_CODE_OAUTH_TOKEN', value: config.claudeOAuthToken }]
        : []),
    ]

    // Merge template with task-specific values
    const podSpec: k8s.V1Pod = {
      ...template,
      metadata: {
        ...template.metadata,
        name: podName,
        namespace,
        labels: {
          ...template.metadata?.labels,
          'moatless.io/task-id': taskId,
        },
        annotations: {
          ...template.metadata?.annotations,
          'moatless.io/repository-url': config.repositoryCloneUrl,
          'moatless.io/created-at': new Date().toISOString(),
        },
      },
      spec: {
        ...template.spec,
        containers: (template.spec?.containers || []).map((container) => {
          if (container.name === 'worker') {
            return {
              ...container,
              env: [...(container.env || []), ...taskEnvVars],
            }
          }
          return container
        }),
      },
    }

    return podSpec
  }

  /**
   * Build pod spec from settings (legacy method)
   */
  private buildPodFromSettings(
    taskId: string,
    podName: string,
    namespace: string,
    config: PodCreationConfig,
    k8sSettings: ReturnType<SettingsService['getKubernetesSettings']>,
  ): k8s.V1Pod {
    // Build resource limits/requests, filtering out undefined values
    const buildResourceMap = (
      cpu: string | undefined,
      memory: string | undefined,
    ): { [key: string]: string } | undefined => {
      const result: { [key: string]: string } = {}
      if (cpu) {
        result['cpu'] = cpu
      }
      if (memory) {
        result['memory'] = memory
      }
      return Object.keys(result).length > 0 ? result : undefined
    }

    // Build tolerations array with proper type handling
    const buildTolerations = (): k8s.V1Toleration[] | undefined => {
      if (!k8sSettings.tolerations) {
        return undefined
      }
      return k8sSettings.tolerations.map((t): k8s.V1Toleration => {
        const toleration: k8s.V1Toleration = {
          key: t.key,
          operator: t.operator,
          effect: t.effect,
        }
        if (t.value !== undefined) {
          toleration.value = t.value
        }
        if (t.tolerationSeconds !== undefined) {
          toleration.tolerationSeconds = t.tolerationSeconds
        }
        return toleration
      })
    }

    // Build resources object avoiding undefined properties
    const resourceLimits = buildResourceMap(
      k8sSettings.resourceLimits?.cpu,
      k8sSettings.resourceLimits?.memory,
    )
    const resourceRequests = buildResourceMap(
      k8sSettings.resourceRequests?.cpu,
      k8sSettings.resourceRequests?.memory,
    )

    // Build container spec
    const container: k8s.V1Container = {
      name: 'worker',
      image: k8sSettings.image,
      imagePullPolicy: 'Always',
      ports: [
        {
          containerPort: 8001,
          name: 'http',
          protocol: 'TCP',
        },
      ],
      env: [
        { name: 'WORKER_MODE', value: 'true' },
        { name: 'WORKER_TASK_ID', value: taskId },
        { name: 'WORKER_API_URL', value: config.workerApiUrl },
        { name: 'MCP_SERVER_URL', value: config.workerApiUrl }, // MCP endpoint on main API
        { name: 'WORKER_AUTH_TOKEN', value: config.workerAuthToken },
        { name: 'GIT_CLONE_URL', value: config.repositoryCloneUrl },
        ...(config.repositoryBranch
          ? [{ name: 'GIT_BRANCH', value: config.repositoryBranch }]
          : []),
        ...(config.githubToken ? [{ name: 'GITHUB_TOKEN', value: config.githubToken }] : []),
        ...(config.anthropicApiKey
          ? [{ name: 'ANTHROPIC_API_KEY', value: config.anthropicApiKey }]
          : []),
        ...(config.claudeOAuthToken
          ? [{ name: 'CLAUDE_CODE_OAUTH_TOKEN', value: config.claudeOAuthToken }]
          : []),
      ],
      readinessProbe: {
        httpGet: {
          path: '/health',
          port: 8001,
        },
        initialDelaySeconds: 5,
        periodSeconds: 5,
        timeoutSeconds: 3,
        failureThreshold: 3,
      },
      livenessProbe: {
        httpGet: {
          path: '/health',
          port: 8001,
        },
        initialDelaySeconds: 30,
        periodSeconds: 10,
        timeoutSeconds: 5,
        failureThreshold: 3,
      },
    }

    // Add resources only if defined
    if (resourceLimits || resourceRequests) {
      const resources: k8s.V1ResourceRequirements = {}
      if (resourceLimits) {
        resources.limits = resourceLimits
      }
      if (resourceRequests) {
        resources.requests = resourceRequests
      }
      container.resources = resources
    }

    // Build pod spec avoiding undefined properties
    const podSpecData: k8s.V1PodSpec = {
      restartPolicy: 'Never',
      containers: [container],
    }
    if (k8sSettings.serviceAccount) {
      podSpecData.serviceAccountName = k8sSettings.serviceAccount
    }
    if (k8sSettings.imagePullSecrets && k8sSettings.imagePullSecrets.length > 0) {
      podSpecData.imagePullSecrets = k8sSettings.imagePullSecrets.map((name) => ({ name }))
    }
    if (k8sSettings.nodeSelector) {
      podSpecData.nodeSelector = k8sSettings.nodeSelector
    }
    const tolerations = buildTolerations()
    if (tolerations) {
      podSpecData.tolerations = tolerations
    }

    return {
      apiVersion: 'v1',
      kind: 'Pod',
      metadata: {
        name: podName,
        namespace,
        labels: {
          'app.kubernetes.io/name': 'moatless-worker',
          'app.kubernetes.io/component': 'worker',
          'app.kubernetes.io/managed-by': 'moatless',
          'moatless.io/task-id': taskId,
        },
        annotations: {
          'moatless.io/repository-url': config.repositoryCloneUrl,
          'moatless.io/created-at': new Date().toISOString(),
        },
      },
      spec: podSpecData,
    }
  }

  /**
   * Delete a pod by task ID
   */
  async deletePod(taskId: string): Promise<void> {
    this.ensureInitialized()

    const k8sSettings = this.settingsService.getKubernetesSettings()
    const namespace = k8sSettings.namespace

    logger.info({ taskId, namespace }, 'KubernetesService: Deleting pod for task')

    try {
      // Find pod by label
      const pods = await this.coreApi!.listNamespacedPod({
        namespace,
        labelSelector: `moatless.io/task-id=${taskId}`,
      })

      if (!pods.items || pods.items.length === 0) {
        logger.warn({ taskId }, 'KubernetesService: No pod found for task')
        return
      }

      for (const pod of pods.items) {
        if (pod.metadata?.name) {
          await this.coreApi!.deleteNamespacedPod({
            name: pod.metadata.name,
            namespace,
            gracePeriodSeconds: 30,
          })
          logger.info({ taskId, podName: pod.metadata.name }, 'KubernetesService: Pod deleted')
        }
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      logger.error({ err: error as Error, taskId }, 'KubernetesService: Failed to delete pod')
      throw new Error(`Failed to delete pod: ${message}`)
    }
  }

  /**
   * Get pod status for a task
   */
  async getPodStatus(taskId: string): Promise<PodStatusInfo | null> {
    this.ensureInitialized()

    const k8sSettings = this.settingsService.getKubernetesSettings()
    const namespace = k8sSettings.namespace

    try {
      const pods = await this.coreApi!.listNamespacedPod({
        namespace,
        labelSelector: `moatless.io/task-id=${taskId}`,
      })

      if (!pods.items || pods.items.length === 0) {
        return null
      }

      const pod = pods.items[0]
      if (!pod) {
        return null
      }

      const status = pod.status

      // Build conditions array, filtering undefined and only including defined properties
      const conditions = status?.conditions
        ?.filter((c): c is k8s.V1PodCondition & { type: string; status: string } =>
          Boolean(c.type && c.status),
        )
        .map((c) => {
          const condition: { type: string; status: string; reason?: string; message?: string } = {
            type: c.type,
            status: c.status,
          }
          if (c.reason !== undefined) {
            condition.reason = c.reason
          }
          if (c.message !== undefined) {
            condition.message = c.message
          }
          return condition
        })

      // Build container statuses
      const containerStatuses = status?.containerStatuses?.map((cs) => ({
        name: cs.name ?? '',
        ready: cs.ready ?? false,
        restartCount: cs.restartCount ?? 0,
        state: cs.state?.running
          ? 'running'
          : cs.state?.waiting
            ? `waiting: ${cs.state.waiting.reason}`
            : cs.state?.terminated
              ? `terminated: ${cs.state.terminated.reason}`
              : 'unknown',
      }))

      const result: PodStatusInfo = {
        phase: (status?.phase ?? 'Unknown') as PodPhase,
      }
      if (status?.podIP) {
        result.podIp = status.podIP
      }
      if (conditions) {
        result.conditions = conditions
      }
      if (containerStatuses) {
        result.containerStatuses = containerStatuses
      }
      return result
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'KubernetesService: Failed to get pod status')
      return null
    }
  }

  /**
   * Get pod IP address for a task
   */
  async getPodIp(taskId: string): Promise<string | null> {
    const status = await this.getPodStatus(taskId)
    return status?.podIp ?? null
  }

  /**
   * Wait for pod to be ready
   */
  async waitForPodReady(taskId: string, timeoutMs?: number): Promise<PodStatusInfo> {
    const k8sSettings = this.settingsService.getKubernetesSettings()
    const timeout = timeoutMs ?? k8sSettings.podStartupTimeoutSeconds * 1000
    const startTime = Date.now()
    const pollInterval = 2000 // 2 seconds

    logger.info({ taskId, timeoutMs: timeout }, 'KubernetesService: Waiting for pod to be ready')

    while (Date.now() - startTime < timeout) {
      const status = await this.getPodStatus(taskId)

      if (!status) {
        logger.debug({ taskId }, 'KubernetesService: Pod not found yet')
        await this.sleep(pollInterval)
        continue
      }

      logger.debug(
        { taskId, phase: status.phase, podIp: status.podIp },
        'KubernetesService: Pod status check',
      )

      // Check if pod failed
      if (status.phase === 'Failed' || status.phase === 'Succeeded') {
        const errorMessage = `Pod entered ${status.phase} phase`
        logger.error({ taskId, status }, `KubernetesService: ${errorMessage}`)
        throw new WorkspaceError(errorMessage, taskId, 503)
      }

      // Check if pod is running and has an IP
      if (status.phase === 'Running' && status.podIp) {
        // Check if container is ready
        const workerContainer = status.containerStatuses?.find((cs) => cs.name === 'worker')
        if (workerContainer?.ready) {
          logger.info({ taskId, podIp: status.podIp }, 'KubernetesService: Pod is ready')
          return status
        }
      }

      await this.sleep(pollInterval)
    }

    throw new WorkspaceError(`Timeout waiting for pod to be ready after ${timeout}ms`, taskId, 503)
  }

  /**
   * Check if worker pod is healthy via HTTP health endpoint
   */
  async checkPodHealth(podIp: string, port = 8001): Promise<boolean> {
    try {
      const response = await fetch(`http://${podIp}:${port}/health`, {
        method: 'GET',
        signal: AbortSignal.timeout(5000),
      })
      return response.ok
    } catch {
      return false
    }
  }

  /**
   * Get pod logs for debugging
   */
  async getPodLogs(taskId: string, tailLines = 100): Promise<string | null> {
    this.ensureInitialized()

    const k8sSettings = this.settingsService.getKubernetesSettings()
    const namespace = k8sSettings.namespace

    try {
      const pods = await this.coreApi!.listNamespacedPod({
        namespace,
        labelSelector: `moatless.io/task-id=${taskId}`,
      })

      if (!pods.items || pods.items.length === 0) {
        return null
      }

      const pod = pods.items[0]
      const podName = pod?.metadata?.name
      if (!podName) {
        return null
      }

      const logs = await this.coreApi!.readNamespacedPodLog({
        name: podName,
        namespace,
        container: 'worker',
        tailLines,
      })

      return logs ?? null
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'KubernetesService: Failed to get pod logs')
      return null
    }
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
