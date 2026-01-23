import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { WorkspaceRuntimeRepository } from '../db/workspace-runtime.repository'
import type { WorkspaceProvisioner } from '../types/workspace-provisioner.types'
import {
  isKubernetesExecutionTarget,
  type WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'
import { WorkspaceRuntimeManager } from './workspace-runtime-manager.service'

/**
 * Background service that suspends workspaces after configurable inactivity.
 *
 * Preserves workspace state (DB snapshot, PVC for K8s) for quick restart
 * on next access via the existing ensureWorkspace() re-provisioning path.
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
    @inject(WorkspaceRuntimeRepository) private readonly runtimeStore: WorkspaceRuntimeRepository,
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

    const snapshots = await this.runtimeStore.list()
    const now = Date.now()

    for (const snapshot of snapshots) {
      try {
        // Skip non-ready workspaces (already stopped/initializing/error)
        if (snapshot.executionTarget.status !== 'ready') {
          continue
        }

        // Skip workspaces with running agents
        if (snapshot.agent?.state === 'running') {
          continue
        }

        // Check inactivity: lastActivityAt → updatedAt fallback
        const lastActivity = snapshot.lastActivityAt ?? snapshot.updatedAt
        if (now - lastActivity.getTime() >= timeoutMs) {
          await this.suspendWorkspace(snapshot)
        }
      } catch (error) {
        this.log.error({ taskId: snapshot.taskId, err: error }, 'Failed to check/suspend workspace')
      }
    }
  }

  private async suspendWorkspace(snapshot: WorkspaceRuntimeSnapshot): Promise<void> {
    this.log.info(
      { taskId: snapshot.taskId, mode: snapshot.executionTarget.mode },
      'Suspending inactive workspace',
    )

    // 1. Stop agent if running (safety check)
    try {
      await this.runtimeManager.stopAgentAndInvalidate(snapshot.taskId)
    } catch (error) {
      this.log.warn({ taskId: snapshot.taskId, err: error }, 'Failed to stop agent during suspend')
    }

    // 2. Stop pod/container
    // K8s: clearError deletes pod only (preserves PVC + secret)
    // Docker: cleanup stops + removes container
    try {
      if (isKubernetesExecutionTarget(snapshot.executionTarget)) {
        await this.provisioner.clearError(snapshot.taskId, snapshot.executionTarget)
      } else {
        await this.provisioner.cleanup(snapshot.taskId, snapshot.executionTarget)
      }
    } catch (error) {
      this.log.warn({ taskId: snapshot.taskId, err: error }, 'Failed to stop workspace resources')
    }

    // 3. Update snapshot status to 'stopped' (keeps snapshot for restart)
    await this.runtimeStore.update(snapshot.taskId, (current) => ({
      ...current,
      executionTarget: { ...current.executionTarget, status: 'stopped' as const },
    }))

    this.log.info({ taskId: snapshot.taskId }, 'Workspace suspended')
  }
}
