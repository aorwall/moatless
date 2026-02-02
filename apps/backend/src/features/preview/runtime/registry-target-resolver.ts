import type { KubernetesService } from '../../../services/kubernetes.service.js'
import { logger } from '../../../utils/logger.js'
import type { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service.js'
import type { ResolveResult, TargetInfo, TargetResolver } from '../types/index.js'

/**
 * Preview server status response from worker pod
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
 * Used by PreviewProxyService to route requests to the correct preview server port.
 * Queries sandbox records for execution target, then falls back to Kubernetes direct query.
 *
 * Resolution strategy:
 * 1. Try sandbox record (fast path) - checks execution target for pod IP + servers
 * 2. If Kubernetes available, query pod directly by label and check preview server status
 *
 * Note: Preview state is NOT persisted. Resolution relies on:
 * - Sandbox record's executionTarget (pod IP, servers map)
 * - Live queries to sandbox/pod for preview server status
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
      // If a specific server is requested (e.g., "files"), resolve its port directly
      if (serverName) {
        return this.resolveServerByName(serverId, serverName)
      }

      // 1. Try sandbox record + live query (fast path) — default: preview server
      const record = await this.runtimeManager.getSandboxRecord(serverId)

      if (!record) {
        this.serviceLogger.debug(
          { serverId },
          'No sandbox record found for serverId, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }

      // Query preview server status live from sandbox
      const handle = await this.runtimeManager.getExistingWorkspaceHandle(serverId)
      if (!handle?.driver.getPreviewServerStatus) {
        this.serviceLogger.debug(
          { serverId },
          'No driver or getPreviewServerStatus available, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }

      try {
        const previewStatus = await handle.driver.getPreviewServerStatus()
        if (previewStatus.status !== 'started' || !previewStatus.port) {
          this.serviceLogger.debug(
            { serverId, previewStatus: previewStatus.status },
            'Preview not started from live query, trying Kubernetes fallback',
          )
          return this.resolveFromKubernetes(serverId)
        }

        const targetUrl = previewStatus.externalUrl || undefined

        this.serviceLogger.debug(
          { serverId, port: previewStatus.port, targetUrl },
          'Successfully resolved target from live query',
        )

        // Mark workspace as active - user viewing preview = real activity
        this.runtimeManager.touchActivity(serverId)

        return {
          port: previewStatus.port,
          targetUrl,
        }
      } catch {
        this.serviceLogger.debug(
          { serverId },
          'Live preview query failed, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }
    } catch (error) {
      this.serviceLogger.warn(
        {
          serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'RegistryTargetResolver failed to resolve target',
      )
      // Try Kubernetes fallback on error
      return this.resolveFromKubernetes(serverId)
    }
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Multi-server routing adds branching to existing complex resolution logic
  async resolveWithContext(serverId: string, serverName?: string): Promise<ResolveResult> {
    // If a specific server is requested, resolve its port directly from the servers map
    if (serverName) {
      const target = await this.resolveServerByName(serverId, serverName)
      if (target) {
        return { ok: true, target }
      }
      // Fall through to check workspace status for proper error messages
    }

    try {
      const record = await this.runtimeManager.getSandboxRecord(serverId)

      if (!record) {
        // No record - try Kubernetes fallback
        const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
        if (k8sResult) {
          return k8sResult
        }
        return { ok: false, reason: 'no_workspace', subdomain: serverId }
      }

      // Check execution target status
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

      // Query preview server status live from sandbox
      const handle = await this.runtimeManager.getExistingWorkspaceHandle(serverId)
      if (!handle?.driver.getPreviewServerStatus) {
        const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
        if (k8sResult) {
          return k8sResult
        }
        return { ok: false, reason: 'preview_not_started', subdomain: serverId }
      }

      try {
        const previewStatus = await handle.driver.getPreviewServerStatus()

        if (previewStatus.status === 'stopped' || !previewStatus.status) {
          const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
          if (k8sResult) {
            return k8sResult
          }
          return { ok: false, reason: 'preview_not_started', subdomain: serverId }
        }

        if (previewStatus.status === 'starting' || previewStatus.status === 'installing') {
          return { ok: false, reason: 'preview_starting', subdomain: serverId }
        }

        if (previewStatus.status !== 'started' || !previewStatus.port) {
          const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
          if (k8sResult) {
            return k8sResult
          }
          return { ok: false, reason: 'preview_not_started', subdomain: serverId }
        }

        const targetUrl = previewStatus.externalUrl || undefined

        this.serviceLogger.debug(
          { serverId, port: previewStatus.port, targetUrl },
          'resolveWithContext: resolved from live query',
        )

        this.runtimeManager.touchActivity(serverId)

        return {
          ok: true,
          target: { port: previewStatus.port, targetUrl },
        }
      } catch {
        const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
        if (k8sResult) {
          return k8sResult
        }
        return { ok: false, reason: 'preview_not_started', subdomain: serverId }
      }
    } catch (error) {
      this.serviceLogger.warn(
        {
          serverId,
          error: error instanceof Error ? error.message : String(error),
        },
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
   * Resolve a specific named server from the execution target's servers map.
   * Used for non-preview servers like file-explorer (routed via task-123--files.proxy.domain).
   *
   * The server port is looked up from executionTarget.servers[serverName].
   * The target URL points directly to the pod IP + server port (no sandbox proxy needed).
   */
  private async resolveServerByName(
    taskId: string,
    serverName: string,
  ): Promise<TargetInfo | null> {
    const record = await this.runtimeManager.getSandboxRecord(taskId)

    if (!record) {
      return null
    }

    // Look up server port from execution target servers map
    const serverEntry = record.executionTarget?.servers?.[serverName]
    if (!serverEntry) {
      this.serviceLogger.debug(
        {
          taskId,
          serverName,
          availableServers: Object.keys(record.executionTarget?.servers ?? {}),
        },
        'Server not found in execution target servers map',
      )
      return null
    }

    // For Kubernetes targets, build direct URL to pod IP + server port
    if (record.executionTarget.mode === 'kubernetes') {
      const k8sTarget = record.executionTarget
      if (k8sTarget.podIp) {
        const targetUrl = `http://${k8sTarget.podIp}:${serverEntry.port}`
        this.runtimeManager.touchActivity(taskId)
        return { port: serverEntry.port, targetUrl }
      }
    }

    // For Docker or other modes, just return the port
    this.runtimeManager.touchActivity(taskId)
    return { port: serverEntry.port }
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
      // Query pod by label
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

      // Parse port from internalUrl if port is not directly provided
      // internalUrl format: "http://localhost:3000"
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

      // Note: port is optional since we use targetUrl for the full proxy path
      // port is only used as fallback when targetUrl is not provided
      // Note: scriptContent is no longer returned - sandbox handles script injection via /preview endpoint
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
