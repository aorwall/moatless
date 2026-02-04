import type { KubernetesService } from '../../../services/kubernetes.service.js'
import { logger } from '../../../utils/logger.js'
import type { SandboxRecord } from '../../workspace/db/workspace-sandbox.schema.js'
import type { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service.js'
import {
  isDockerExecutionTarget,
  isKubernetesExecutionTarget,
} from '../../workspace/types/workspace-runtime.types.js'
import type {
  ResolutionFailureReason,
  ResolveResult,
  TargetInfo,
  TargetResolver,
} from '../types/index.js'

/**
 * Preview server status response from worker pod (used only in K8s fallback)
 */
interface PreviewServerStatus {
  status: 'stopped' | 'starting' | 'started' | 'error'
  port?: number
  internalUrl?: string // e.g., "http://localhost:3000"
  command?: string
  directory?: string
  error?: string
}

/**
 * RegistryTargetResolver - Resolves preview server targets from sandbox records
 *
 * Used by HostnameProxyService to route requests to the correct preview server.
 *
 * Resolution strategy (fast path — no live queries):
 * 1. Look up sandbox record for the task
 * 2. Build target URL from execution target (pod IP / container name + port)
 * 3. Return immediately — the proxy attempt itself is the real health check
 *
 * On proxy error, the hostname proxy calls getServerStatus() lazily to pick
 * the right error page (starting, stopped, connection refused, etc.).
 *
 * Kubernetes direct query is kept as fallback when no sandbox record exists.
 */
export class RegistryTargetResolver implements TargetResolver {
  private readonly serviceLogger: typeof logger

  constructor(
    private readonly runtimeManager: WorkspaceRuntimeManager,
    private readonly kubernetesService?: KubernetesService,
  ) {
    this.serviceLogger = logger.child({ service: 'registry-target-resolver' })
  }

  async resolve(serverId: string, serverName?: string): Promise<TargetInfo | null> {
    try {
      const record = await this.runtimeManager.getSandboxRecord(serverId)

      if (!record) {
        this.serviceLogger.debug(
          { serverId, serverName },
          'No sandbox record found, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }

      const target = this.buildTargetFromRecord(record, serverName)
      if (target) {
        this.serviceLogger.debug(
          {
            serverId,
            serverName,
            port: target.port,
            targetUrl: target.targetUrl,
            mode: record.executionTarget.mode,
          },
          'Resolved target from sandbox record',
        )
        this.runtimeManager.touchActivity(serverId)
        return target
      }

      // Could not build target from record — K8s fallback or null
      this.serviceLogger.debug(
        {
          serverId,
          serverName,
          mode: record.executionTarget.mode,
          execStatus: record.executionTarget.status,
        },
        'Could not build target from record, trying Kubernetes fallback',
      )
      return this.resolveFromKubernetes(serverId)
    } catch (error) {
      this.serviceLogger.warn(
        { serverId, serverName, error: error instanceof Error ? error.message : String(error) },
        'resolve() failed, trying Kubernetes fallback',
      )
      return this.resolveFromKubernetes(serverId)
    }
  }

  async resolveWithContext(serverId: string, serverName?: string): Promise<ResolveResult> {
    try {
      const record = await this.runtimeManager.getSandboxRecord(serverId)

      if (!record) {
        const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
        if (k8sResult) {
          return k8sResult
        }
        return { ok: false, reason: 'no_workspace', subdomain: serverId }
      }

      // Check execution target status — blocked states return immediately
      const execStatus = record.executionTarget?.status
      if (execStatus === 'stopped') {
        return {
          ok: false,
          reason: 'workspace_stopped',
          subdomain: serverId,
          autoStart: () => {
            this.runtimeManager.ensureWorkspace(serverId).catch((err) => {
              this.serviceLogger.warn(
                { serverId, error: err instanceof Error ? err.message : String(err) },
                'Auto-start workspace failed',
              )
            })
          },
        }
      }

      if (execStatus === 'initializing') {
        return { ok: false, reason: 'workspace_initializing', subdomain: serverId }
      }

      // Fast path: build target from execution record (no live queries)
      const target = this.buildTargetFromRecord(record, serverName)
      if (target) {
        this.serviceLogger.debug(
          {
            serverId,
            serverName,
            port: target.port,
            targetUrl: target.targetUrl,
            mode: record.executionTarget.mode,
          },
          'resolveWithContext: resolved from sandbox record',
        )
        this.runtimeManager.touchActivity(serverId)
        return { ok: true, target }
      }

      // Could not build target — K8s fallback or contextual error
      const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
      if (k8sResult) {
        return k8sResult
      }

      this.serviceLogger.debug(
        { serverId, serverName, mode: record.executionTarget.mode, execStatus },
        'resolveWithContext: no target and no K8s fallback',
      )
      return { ok: false, reason: 'preview_not_started', subdomain: serverId }
    } catch (error) {
      this.serviceLogger.warn(
        { serverId, serverName, error: error instanceof Error ? error.message : String(error) },
        'resolveWithContext failed',
      )
      const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
      if (k8sResult) {
        return k8sResult
      }
      return { ok: false, reason: 'no_workspace', subdomain: serverId }
    }
  }

  /**
   * Lazy status check — called only on proxy error to determine the right error page.
   * Queries the sandbox for preview server status and maps to a ResolutionFailureReason.
   */
  async getServerStatus(
    taskId: string,
    _serverName?: string,
  ): Promise<ResolutionFailureReason | null> {
    const handle = await this.runtimeManager.getExistingWorkspaceHandle(taskId)
    if (!handle?.driver.getPreviewServerStatus) {
      return null
    }

    try {
      const status = await handle.driver.getPreviewServerStatus()
      if (status.status === 'starting' || status.status === 'installing') {
        return 'preview_starting'
      }
      if (status.status === 'stopped' || !status.status) {
        return 'preview_not_started'
      }
      if (status.status === 'started') {
        return 'connection_refused'
      }
      return null
    } catch {
      return null
    }
  }

  /**
   * Build a TargetInfo from a sandbox record's execution target.
   * Returns null if the record doesn't have enough info to build a URL.
   */
  private buildTargetFromRecord(record: SandboxRecord, serverName?: string): TargetInfo | null {
    const target = record.executionTarget
    const port = serverName
      ? target.servers?.[serverName]
      : (target.servers?.['preview'] ?? target.previewHostPort ?? 3000)

    if (!port) {
      return null
    }

    if (isKubernetesExecutionTarget(target)) {
      if (!target.podIp) {
        return null
      }
      return { port, targetUrl: `http://${target.podIp}:${port}` }
    }
    if (isDockerExecutionTarget(target)) {
      if (!target.containerName) {
        return null
      }
      return { port, targetUrl: `http://${target.containerName}:${port}` }
    }
    return { port }
  }

  /**
   * Kubernetes fallback for resolveWithContext - returns ResolveResult or null
   */
  private async resolveFromKubernetesWithContext(taskId: string): Promise<ResolveResult | null> {
    if (!this.kubernetesService) {
      return null
    }

    try {
      const pod = await this.kubernetesService.findExistingPod(taskId)

      if (!pod) {
        return null
      }

      if (pod.phase !== 'Running') {
        return { ok: false, reason: 'pod_not_running', subdomain: taskId }
      }

      if (!pod.podIp) {
        return { ok: false, reason: 'pod_not_running', subdomain: taskId }
      }

      const workerApiUrl = `http://${pod.podIp}:8001`
      const status = await this.queryPodPreviewStatus(workerApiUrl, taskId)

      if (!status || status.status !== 'started') {
        const reason = status?.status === 'starting' ? 'preview_starting' : 'preview_not_started'
        return { ok: false, reason, subdomain: taskId }
      }

      let port = status.port
      if (!port && status.internalUrl) {
        try {
          const url = new URL(status.internalUrl)
          port = Number.parseInt(url.port, 10) || (url.protocol === 'https:' ? 443 : 80)
        } catch {
          // Ignore URL parse errors
        }
      }

      const resolvedPort = port || 3000
      const targetUrl = `http://${pod.podIp}:${resolvedPort}`

      return {
        ok: true,
        target: { port: resolvedPort, targetUrl },
      }
    } catch {
      return null
    }
  }

  /**
   * Resolve target by querying Kubernetes directly
   *
   * This is the fallback when sandbox record is stale or missing.
   * Queries pod by label (moatless.io/task-id), then checks preview server status.
   */
  private async resolveFromKubernetes(taskId: string): Promise<TargetInfo | null> {
    if (!this.kubernetesService) {
      this.serviceLogger.debug(
        { taskId },
        'Kubernetes service not available, cannot fallback to direct pod query',
      )
      return null
    }

    try {
      const pod = await this.kubernetesService.findExistingPod(taskId)

      if (!pod) {
        this.serviceLogger.debug({ taskId }, 'No pod found in Kubernetes for taskId')
        return null
      }

      if (pod.phase !== 'Running') {
        this.serviceLogger.debug(
          { taskId, podName: pod.podName, phase: pod.phase },
          'Pod found but not in Running phase',
        )
        return null
      }

      if (!pod.podIp) {
        this.serviceLogger.debug(
          { taskId, podName: pod.podName, phase: pod.phase },
          'Pod is Running but has no IP assigned',
        )
        return null
      }

      // Query preview server status on pod
      const workerApiUrl = `http://${pod.podIp}:8001`
      const status = await this.queryPodPreviewStatus(workerApiUrl, taskId)

      if (!status) {
        this.serviceLogger.debug(
          { taskId, podName: pod.podName, podIp: pod.podIp },
          'Could not get preview server status from pod',
        )
        return null
      }

      if (status.status !== 'started') {
        this.serviceLogger.debug(
          { taskId, podName: pod.podName, previewStatus: status.status },
          'Preview server on pod is not started',
        )
        return null
      }

      let port = status.port
      if (!port && status.internalUrl) {
        try {
          const url = new URL(status.internalUrl)
          port = Number.parseInt(url.port, 10) || (url.protocol === 'https:' ? 443 : 80)
        } catch {
          // Ignore URL parse errors
        }
      }

      const resolvedPort = port || 3000
      const targetUrl = `http://${pod.podIp}:${resolvedPort}`

      this.serviceLogger.debug(
        { taskId, podName: pod.podName, podIp: pod.podIp, port: resolvedPort },
        'Successfully resolved target from Kubernetes direct query',
      )

      return {
        port: resolvedPort,
        targetUrl,
      }
    } catch (error) {
      this.serviceLogger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to resolve target from Kubernetes',
      )
      return null
    }
  }

  /**
   * Query preview server status from worker pod API
   */
  private async queryPodPreviewStatus(
    workerApiUrl: string,
    taskId: string,
  ): Promise<PreviewServerStatus | null> {
    try {
      const response = await fetch(`${workerApiUrl}/server/status`, {
        signal: AbortSignal.timeout(3000),
      })

      if (!response.ok) {
        this.serviceLogger.debug(
          { taskId, workerApiUrl, status: response.status },
          'Preview server status request failed',
        )
        return null
      }

      const status = (await response.json()) as PreviewServerStatus
      return status
    } catch (error) {
      this.serviceLogger.debug(
        {
          taskId,
          workerApiUrl,
          error: error instanceof Error ? error.message : String(error),
        },
        'Error querying preview server status',
      )
      return null
    }
  }
}
