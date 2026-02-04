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
 * RegistryTargetResolver - Resolves preview server targets from sandbox records
 *
 * Used by HostnameProxyService to route requests to the correct preview server.
 *
 * Resolution strategy (fast path — no live queries):
 * 1. Look up sandbox record for the task
 * 2. Build target URL from execution target (Service DNS / container name + port)
 * 3. Return immediately — the proxy attempt itself is the real health check
 *
 * On proxy error, the hostname proxy calls getServerStatus() lazily to pick
 * the right error page (starting, stopped, connection refused, etc.).
 */
export class RegistryTargetResolver implements TargetResolver {
  private readonly serviceLogger: typeof logger

  constructor(private readonly runtimeManager: WorkspaceRuntimeManager) {
    this.serviceLogger = logger.child({ service: 'registry-target-resolver' })
  }

  async resolve(serverId: string, serverName?: string): Promise<TargetInfo | null> {
    try {
      const record = await this.runtimeManager.getSandboxRecord(serverId)

      if (!record) {
        this.serviceLogger.debug({ serverId, serverName }, 'No sandbox record found')
        return null
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

      this.serviceLogger.debug(
        {
          serverId,
          serverName,
          mode: record.executionTarget.mode,
          execStatus: record.executionTarget.status,
        },
        'Could not build target from record',
      )
      return null
    } catch (error) {
      this.serviceLogger.warn(
        { serverId, serverName, error: error instanceof Error ? error.message : String(error) },
        'resolve() failed',
      )
      return null
    }
  }

  async resolveWithContext(serverId: string, serverName?: string): Promise<ResolveResult> {
    try {
      const record = await this.runtimeManager.getSandboxRecord(serverId)

      if (!record) {
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
        // serviceName is always set (required field), but the Service DNS won't resolve
        // until the pod has endpoints. The proxy will get a connection error, triggering
        // getServerStatus() which shows the appropriate "workspace initializing" page.
        const target = this.buildTargetFromRecord(record, serverName)
        if (target) {
          this.runtimeManager.touchActivity(serverId)
          return { ok: true, target }
        }
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

      this.serviceLogger.debug(
        { serverId, serverName, mode: record.executionTarget.mode, execStatus },
        'resolveWithContext: no target from record',
      )
      return { ok: false, reason: 'preview_not_started', subdomain: serverId }
    } catch (error) {
      this.serviceLogger.warn(
        { serverId, serverName, error: error instanceof Error ? error.message : String(error) },
        'resolveWithContext failed',
      )
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
      return {
        port,
        targetUrl: `http://${target.serviceName}.${target.podNamespace}.svc.cluster.local:${port}`,
      }
    }
    if (isDockerExecutionTarget(target)) {
      if (!target.containerName) {
        return null
      }
      return { port, targetUrl: `http://${target.containerName}:${port}` }
    }
    return { port }
  }
}
