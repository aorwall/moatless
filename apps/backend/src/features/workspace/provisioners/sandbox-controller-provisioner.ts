import type { EnvironmentStatus, WorkspaceStatusResponse } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import { RemoteEnvironmentDriver } from '../drivers/remote-environment-driver'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type { ExecutionTarget } from '../types/workspace-runtime.types'
import { buildServersConfig } from '../utils/server-ports'

/**
 * Result of getStatus() call - provides detailed workspace status for logging
 */
export interface WorkspaceStatusResult {
  status: EnvironmentStatus
  podName?: string | undefined
  lastError?: string | undefined
}

/**
 * Base URL for the sandbox controller API.
 * Defaults to port 8080 (unified API surface).
 */
function getSandboxControllerUrl(): string {
  return process.env['SANDBOX_CONTROLLER_URL'] || 'http://localhost:8080'
}

/**
 * SandboxControllerProvisioner - Thin HTTP adapter to sandbox controller API
 *
 * Delegates all Kubernetes operations (pod/PVC/secret/service CRUD) to the
 * sandbox controller. The TS backend retains all state management, event
 * emission, and orchestration.
 *
 * Each method maps to a single HTTP call to the sandbox controller API.
 */
@injectable()
export class SandboxControllerProvisioner {
  private readonly baseUrl = getSandboxControllerUrl()

  /**
   * Create an initializing execution target before provisioning begins.
   * Pure local computation — no HTTP call needed.
   */
  createInitializingTarget(taskId: string, plan: WorkspaceProvisionPlan): ExecutionTarget {
    const namespace = process.env['K8S_NAMESPACE'] || process.env['SANDBOX_NAMESPACE'] || 'default'

    // Deterministic service name (must match Rust's generate_service_name)
    const sanitized = taskId
      .toLowerCase()
      .replace(/[^a-z0-9-]/g, '-')
      .substring(0, 49)
    const serviceName = `moatless-ws-${sanitized}`.substring(0, 63)

    // Build servers map
    const serversWithPorts = buildServersConfig(plan.repoConfig)
    const serversMap: Record<string, number> = {}
    for (const s of serversWithPorts) {
      serversMap[s.name] = s.port
    }

    const target: ExecutionTarget = {
      mode: 'kubernetes',
      podNamespace: namespace,
      serviceName,
      status: 'initializing',
      repositoryCloneUrl: plan.gitCloneUrl,
      sandboxPort: 8001,
      previewHostPort: 3000,
      servers: serversMap,
    }
    if (plan.gitBranch) {
      target.repositoryBranch = plan.gitBranch
    }
    return target
  }

  /**
   * Provision a workspace via sandbox controller internal API.
   */
  async provision(taskId: string, plan: WorkspaceProvisionPlan): Promise<ExecutionTarget> {
    const { SessionContext } = await import('../../auth/services/session-context')
    const session = SessionContext.getCurrentSession()
    if (!session) {
      throw new Error('Cannot provision workspace: No authenticated user session.')
    }

    // Build servers config for the request
    const serversWithPorts = buildServersConfig(plan.repoConfig)

    const requestBody = {
      plan: {
        repoConfig: {
          name: plan.repoConfig.name,
          dockerImage: plan.repoConfig.codeExecution?.dockerImage,
          servers: serversWithPorts.map((s) => ({ name: s.name, port: s.port })),
          pluginIds: plan.repoConfig.plugins,
          // Pass-through fields for sandbox REPO_CONFIG env var
          systemPrompt: plan.repoConfig.systemPrompt,
          codeExecution: plan.repoConfig.codeExecution,
          previewConfig: plan.repoConfig.previewConfig,
        },
        gitCloneUrl: plan.gitCloneUrl,
        gitBranch: plan.gitBranch,
        gitProviderToken: plan.gitProviderToken || undefined,
        claudeOAuthToken: plan.claudeOAuthToken || undefined,
        anthropicApiKey: plan.anthropicApiKey,
        envVars: plan.repoConfig.codeExecution?.envVars,
      },
      userId: session.userId,
    }

    const url = `${this.baseUrl}/api/sandbox/v1/tasks/${encodeURIComponent(taskId)}/provision`
    logger.info(
      { taskId, url },
      'SandboxControllerProvisioner: Provisioning workspace via sandbox controller',
    )

    const response = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(requestBody),
      signal: AbortSignal.timeout(180_000), // 3 minute timeout (provisioning can be slow)
    })

    if (!response.ok) {
      const body = await response.text()
      throw new Error(`Sandbox controller provisioning failed (${response.status}): ${body}`)
    }

    const data = (await response.json()) as {
      executionTarget: {
        mode: string
        podName?: string
        podNamespace: string
        serviceName: string
        podPhase?: string
        status: string
        repositoryCloneUrl: string
        repositoryBranch?: string
        sandboxPort: number
        previewHostPort: number
        servers: Record<string, number>
      }
    }

    const et = data.executionTarget
    const target: ExecutionTarget = {
      mode: 'kubernetes',
      podName: et.podName,
      podNamespace: et.podNamespace,
      serviceName: et.serviceName,
      status: et.status as EnvironmentStatus,
      repositoryCloneUrl: et.repositoryCloneUrl,
      sandboxPort: et.sandboxPort,
      previewHostPort: et.previewHostPort,
      servers: et.servers,
    }
    if (et.repositoryBranch) {
      target.repositoryBranch = et.repositoryBranch
    }

    // IngressRoutes are created by the Rust sandbox controller during provisioning

    logger.info(
      { taskId, status: target.status },
      'SandboxControllerProvisioner: Workspace provisioned',
    )
    return target
  }

  /**
   * Check workspace health via sandbox controller.
   */
  async isHealthy(taskId: string, _target: ExecutionTarget): Promise<boolean> {
    try {
      const url = `${this.baseUrl}/api/sandbox/v1/tasks/${encodeURIComponent(taskId)}/health`
      const response = await fetch(url, { signal: AbortSignal.timeout(10_000) })
      if (!response.ok) {
        return false
      }

      const data = (await response.json()) as { healthy: boolean }
      return data.healthy
    } catch {
      return false
    }
  }

  /**
   * Get workspace status via sandbox controller.
   */
  async getStatus(taskId: string, _target: ExecutionTarget): Promise<WorkspaceStatusResult> {
    try {
      const url = `${this.baseUrl}/api/sandbox/v1/tasks/${encodeURIComponent(taskId)}/env-status`
      const response = await fetch(url, { signal: AbortSignal.timeout(10_000) })
      if (!response.ok) {
        return { status: 'error', lastError: `HTTP ${response.status}` }
      }

      const data = (await response.json()) as {
        status: string
        podName?: string
        lastError?: string
      }
      return {
        status: data.status as EnvironmentStatus,
        podName: data.podName,
        lastError: data.lastError,
      }
    } catch (error) {
      return { status: 'error', lastError: String(error) }
    }
  }

  /**
   * Check if agent is running by querying the sandbox pod directly.
   * This doesn't go through Rust — it queries the sandbox directly via service DNS.
   */
  async isAgentRunning(_taskId: string, target: ExecutionTarget): Promise<boolean> {
    const serviceHost = `${target.serviceName}.${target.podNamespace}.svc.cluster.local`

    try {
      const sandboxUrl = `http://${serviceHost}:${target.sandboxPort}/agent-status`
      const response = await fetch(sandboxUrl, { signal: AbortSignal.timeout(5000) })
      if (!response.ok) {
        return false
      }

      const data = (await response.json()) as { state: string }
      return data.state === 'running'
    } catch {
      return false
    }
  }

  /**
   * Get full workspace status from sandbox controller.
   * Single HTTP call — no DB queries needed.
   */
  async getFullWorkspaceStatus(taskId: string): Promise<WorkspaceStatusResponse> {
    const url = `${this.baseUrl}/api/sandbox/v1/tasks/${encodeURIComponent(taskId)}/status`

    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(10_000) })
      if (!response.ok) {
        const body = await response.text()
        logger.warn(
          { taskId, status: response.status, body },
          'SandboxControllerProvisioner: workspace-status request failed',
        )
        return {
          workspaceState: 'missing',
          environmentStatus: 'not_created',
          agentState: 'none',
          servers: [],
          isRunning: false,
          workspaceReady: false,
          canExecute: false,
        }
      }

      return (await response.json()) as WorkspaceStatusResponse
    } catch (error) {
      logger.warn(
        { taskId, error: String(error) },
        'SandboxControllerProvisioner: workspace-status request error',
      )
      return {
        workspaceState: 'missing',
        environmentStatus: 'not_created',
        agentState: 'none',
        servers: [],
        isRunning: false,
        workspaceReady: false,
        canExecute: false,
      }
    }
  }

  /**
   * Full cleanup via sandbox controller (deletes pod + PVC + secret + service).
   */
  async cleanup(
    taskId: string,
    _target: ExecutionTarget,
    _plan?: WorkspaceProvisionPlan,
  ): Promise<void> {
    const url = `${this.baseUrl}/api/sandbox/v1/tasks/${encodeURIComponent(taskId)}/cleanup`
    logger.info({ taskId }, 'SandboxControllerProvisioner: Cleaning up workspace')

    try {
      await fetch(url, {
        method: 'POST',
        signal: AbortSignal.timeout(30_000),
      })
    } catch (error) {
      logger.warn(
        { taskId, error: String(error) },
        'SandboxControllerProvisioner: Cleanup request failed',
      )
    }
  }

  /**
   * Stop sandbox via sandbox controller (deletes pod + service, keeps PVC + secret).
   */
  async stopSandbox(taskId: string, _target: ExecutionTarget): Promise<void> {
    const url = `${this.baseUrl}/api/sandbox/v1/tasks/${encodeURIComponent(taskId)}/stop`
    logger.info({ taskId }, 'SandboxControllerProvisioner: Stopping workspace')

    try {
      await fetch(url, {
        method: 'POST',
        signal: AbortSignal.timeout(30_000),
      })
    } catch (error) {
      logger.warn(
        { taskId, error: String(error) },
        'SandboxControllerProvisioner: Stop request failed',
      )
    }
  }

  /**
   * Create a RemoteEnvironmentDriver for the workspace.
   * Pure local construction — no HTTP call to Rust.
   */
  createDriver(taskId: string, target: ExecutionTarget): BaseEnvironmentDriver {
    const serviceHost = `${target.serviceName}.${target.podNamespace}.svc.cluster.local`
    const previewPort = target.previewHostPort ?? 3000

    return new RemoteEnvironmentDriver({
      taskId,
      getSandboxUrl: async () => `http://${serviceHost}:${target.sandboxPort}`,
      getPreviewUrl: async () => `http://${serviceHost}:${previewPort}`,
    })
  }
}
