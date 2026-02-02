import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { WorkspaceSandboxRepository } from '../db/workspace-sandbox.repository'
import type { SandboxRecord } from '../db/workspace-sandbox.schema'
import type { WorkspaceProvisioner } from '../types/workspace-provisioner.types'
import { isKubernetesExecutionTarget } from '../types/workspace-runtime.types'
import { WorkspaceRuntimeManager } from './workspace-runtime-manager.service'

/**
 * Background service that suspends workspaces after configurable inactivity.
 *
 * Preserves workspace state (DB sandbox record, PVC for K8s) for quick restart
 * on next access via the existing ensureWorkspace() re-provisioning path.
 *
 * Uses workspace_sandboxes table as sole persistence layer.
 * Agent state is queried live from sandbox (not persisted).
 *
 * Configuration (env vars):
 * - WORKSPACE_INACTIVITY_TIMEOUT_MINUTES: minutes of inactivity before suspend (default 20, 0 = disabled)
 * - WORKSPACE_INACTIVITY_CHECK_INTERVAL_MS: how often to scan (default 60000)
 */
@injectable()
export class WorkspaceInactivityService {
  private checkInterval: ReturnType<typeof setInterval> | null = null
  private readonly log = logger.child({ service: 'workspace-inactivity' })

  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(WorkspaceSandboxRepository) private readonly sandboxStore: WorkspaceSandboxRepository,
    @inject('WorkspaceProvisioner') private readonly provisioner: WorkspaceProvisioner,
  ) {}

  start(checkIntervalMs?: number): void {
    const timeoutMinutes = this.getTimeoutMinutes()
    if (timeoutMinutes <= 0) {
      this.log.info('Workspace inactivity timeout disabled (timeout=0)')
      return
    }

    const intervalMs =
      checkIntervalMs ?? (Number(process.env['WORKSPACE_INACTIVITY_CHECK_INTERVAL_MS']) || 60_000)
    this.log.info({ intervalMs, timeoutMinutes }, 'Starting workspace inactivity checker')
    this.checkInterval = setInterval(() => this.checkInactiveWorkspaces(), intervalMs)
  }

  stop(): void {
    if (this.checkInterval) {
      clearInterval(this.checkInterval)
      this.checkInterval = null
    }
  }

  private getTimeoutMinutes(): number {
    return Number(process.env['WORKSPACE_INACTIVITY_TIMEOUT_MINUTES'] ?? 20)
  }

  private getTimeoutMs(): number {
    return this.getTimeoutMinutes() * 60 * 1000
  }

  async checkInactiveWorkspaces(): Promise<void> {
    const timeoutMs = this.getTimeoutMs()
    if (timeoutMs <= 0) {
      return
    }

    const sandboxRecords = await this.sandboxStore.list()
    const now = Date.now()

    for (const record of sandboxRecords) {
      try {
        // Check inactivity from sandbox record: lastActivityAt → updatedAt fallback
        const lastActivity = record.lastActivityAt ?? record.updatedAt
        if (now - lastActivity.getTime() < timeoutMs) {
          continue // Still active, skip
        }

        // Skip non-ready workspaces (already stopped/initializing/error)
        if (record.executionTarget.status !== 'ready') {
          continue
        }

        // Skip workspaces with running agents (live query)
        try {
          const isRunning = await this.provisioner.isAgentRunning(
            record.taskId,
            record.executionTarget,
          )
          if (isRunning) {
            continue
          }
        } catch {
          // If we can't query, proceed with suspension
        }

        await this.suspendWorkspace(record)
      } catch (error) {
        this.log.error({ taskId: record.taskId, err: error }, 'Failed to check/suspend workspace')
      }
    }
  }

  private async suspendWorkspace(record: SandboxRecord): Promise<void> {
    this.log.info(
      { taskId: record.taskId, mode: record.executionTarget.mode },
      'Suspending inactive workspace',
    )

    // 1. Stop agent if running (safety check)
    try {
      await this.runtimeManager.stopAgentAndInvalidate(record.taskId)
    } catch (error) {
      this.log.warn({ taskId: record.taskId, err: error }, 'Failed to stop agent during suspend')
    }

    // 2. Stop pod/container
    // K8s: clearError deletes pod only (preserves PVC + secret)
    // Docker: cleanup stops + removes container
    try {
      if (isKubernetesExecutionTarget(record.executionTarget)) {
        await this.provisioner.clearError(record.taskId, record.executionTarget)
      } else {
        await this.provisioner.cleanup(record.taskId, record.executionTarget)
      }
    } catch (error) {
      this.log.warn({ taskId: record.taskId, err: error }, 'Failed to stop workspace resources')
    }

    // 3. Update sandbox record status to 'stopped' (keeps record for restart)
    await this.sandboxStore.updateExecutionTarget(record.taskId, {
      ...record.executionTarget,
      status: 'stopped' as const,
    })

    this.log.info({ taskId: record.taskId }, 'Workspace suspended')
  }
}
