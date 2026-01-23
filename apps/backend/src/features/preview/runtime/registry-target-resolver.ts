import type { KubernetesService } from '../../../services/kubernetes.service.js'
import { logger } from '../../../utils/logger.js'
import type { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service.js'
import type { TargetInfo, TargetResolver } from '../types/index.js'

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

  async resolve(serverId: string): Promise<TargetInfo | null> {
    try {
      // 1. Try snapshot first (fast path)
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
      const targetUrl = `${workerApiUrl}/preview`

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
