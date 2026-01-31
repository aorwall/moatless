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
 * RegistryTargetResolver - Resolves preview server targets from workspace snapshots
 *
 * Used by PreviewProxyService to route requests to the correct preview server port.
 * Queries workspace runtime snapshots first, then falls back to Kubernetes direct query.
 *
 * Resolution strategy:
 * 1. Try snapshot (fast path) - checks in-memory/DB cached state
 * 2. If Kubernetes available, query pod directly by label and check preview server status
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

      // 1. Try snapshot first (fast path) — default: preview server
      const snapshot = await this.runtimeManager.getSnapshot(serverId)

      if (!snapshot) {
        this.serviceLogger.debug(
          { serverId },
          'No snapshot found for serverId, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }

      if (!snapshot.preview) {
        this.serviceLogger.debug(
          { serverId, snapshotKeys: Object.keys(snapshot) },
          'Snapshot exists but no preview state, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }

      const { status, port, proxyUrl } = snapshot.preview

      if (status !== 'started') {
        this.serviceLogger.debug(
          { serverId, previewStatus: status },
          'Preview status is not started, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }

      if (!port) {
        this.serviceLogger.debug(
          { serverId, previewStatus: status },
          'Preview started but no port configured, trying Kubernetes fallback',
        )
        return this.resolveFromKubernetes(serverId)
      }

      this.serviceLogger.debug(
        { serverId, port, proxyUrl },
        'Successfully resolved target from snapshot',
      )

      // Mark workspace as active - user viewing preview = real activity
      this.runtimeManager.touchActivity(serverId, snapshot)

      // Note: scriptContent is no longer returned here - sandbox handles script injection
      return {
        port,
        targetUrl: proxyUrl, // Full URL for Kubernetes worker pods (/preview endpoint)
      }
    } catch (error) {
      this.serviceLogger.warn(
        {
          serverId,
          error: error instanceof Error ? error.message : String(error),
        },
        'RegistryTargetResolver failed to resolve target from snapshot',
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
      const snapshot = await this.runtimeManager.getSnapshot(serverId)

      if (!snapshot) {
        // No snapshot - try Kubernetes fallback
        const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
        if (k8sResult) {
          return k8sResult
        }
        return { ok: false, reason: 'no_workspace', subdomain: serverId }
      }

      // Check execution target status
      const execStatus = snapshot.executionTarget?.status
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

      // Workspace exists but preview not started or missing
      if (!snapshot.preview || snapshot.preview.status === 'stopped') {
        // Try Kubernetes fallback first
        const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
        if (k8sResult) {
          return k8sResult
        }
        return { ok: false, reason: 'preview_not_started', subdomain: serverId }
      }

      if (snapshot.preview.status === 'starting' || snapshot.preview.status === 'installing') {
        return { ok: false, reason: 'preview_starting', subdomain: serverId }
      }

      if (snapshot.preview.status !== 'started' || !snapshot.preview.port) {
        // Try Kubernetes fallback
        const k8sResult = await this.resolveFromKubernetesWithContext(serverId)
        if (k8sResult) {
          return k8sResult
        }
        return { ok: false, reason: 'preview_not_started', subdomain: serverId }
      }

      const { port, proxyUrl } = snapshot.preview

      this.serviceLogger.debug(
        { serverId, port, proxyUrl },
        'resolveWithContext: resolved from snapshot',
      )

      this.runtimeManager.touchActivity(serverId, snapshot)

      return {
        ok: true,
        target: { port, targetUrl: proxyUrl },
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
   * The server port is looked up from snapshot.executionTarget.servers[serverName].
   * The target URL points directly to the pod IP + server port (no sandbox proxy needed).
   */
  private async resolveServerByName(
    taskId: string,
    serverName: string,
  ): Promise<TargetInfo | null> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)

    if (!snapshot) {
      return null
    }

    // Look up server port from execution target servers map
    const serverEntry = snapshot.executionTarget?.servers?.[serverName]
    if (!serverEntry) {
      this.serviceLogger.debug(
        {
          taskId,
          serverName,
          availableServers: Object.keys(snapshot.executionTarget?.servers ?? {}),
        },
        'Server not found in execution target servers map',
      )
      return null
    }

    // For Kubernetes targets, build direct URL to pod IP + server port
    if (snapshot.executionTarget.mode === 'kubernetes') {
      const k8sTarget = snapshot.executionTarget
      if (k8sTarget.podIp) {
        const targetUrl = `http://${k8sTarget.podIp}:${serverEntry.port}`
        this.runtimeManager.touchActivity(taskId, snapshot)
        return { port: serverEntry.port, targetUrl }
      }
    }

    // For Docker or other modes, just return the port
    this.runtimeManager.touchActivity(taskId, snapshot)
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

      // Update stale snapshot
      try {
        await this.runtimeManager.updatePreviewState(taskId, {
          status: 'started',
          port: resolvedPort,
          proxyUrl: targetUrl,
        })
      } catch {
        // Non-fatal
      }

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
   * This is the fallback when snapshot is stale or missing.
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

      // Update stale snapshot when K8s shows preview is running
      // This fixes the stale cache for subsequent requests
      try {
        await this.runtimeManager.updatePreviewState(taskId, {
          status: 'started',
          port: resolvedPort,
          proxyUrl: targetUrl,
        })
        this.serviceLogger.debug(
          { taskId, port: resolvedPort },
          'Updated stale preview snapshot from Kubernetes query',
        )
      } catch (updateError) {
        this.serviceLogger.debug(
          {
            taskId,
            error: updateError instanceof Error ? updateError.message : String(updateError),
          },
          'Failed to update preview snapshot (non-fatal)',
        )
      }

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
