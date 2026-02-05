/**
 * RuntimeHealthCollector Service
 *
 * Periodically collects live-verified health metrics for workspaces and tasks.
 * Uses actual API calls to verify resource state rather than just reading database state.
 *
 * Responsibilities:
 * - Query sandbox records from database
 * - Verify K8s pod phase via getValidatedRecord() (queries K8s API)
 * - Update Prometheus gauges with verified state distribution
 *
 * Note: Preview health collection was removed because it requires authenticated user context
 * which is not available in background service execution.
 */

import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceSandboxRepository } from '../../workspace/db/workspace-sandbox.repository'
import type { SandboxRecord } from '../../workspace/db/workspace-sandbox.schema'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { isDockerExecutionTarget } from '../../workspace/types/workspace-runtime.types'
import {
  healthCollectorDuration,
  healthCollectorErrors,
  taskHealth,
  workspaceHealth,
} from '../runtime-health-metrics'

const DOCKER_STATES = ['created', 'running', 'exited', 'removing', 'not_created'] as const
const K8S_PHASES = [
  'Pending',
  'Running',
  'Succeeded',
  'Failed',
  'Unknown',
  'Terminating',
  'NotFound',
] as const
// Combined task statuses: non-open task statuses + iteration statuses for open tasks
// Excludes 'closed' as those are not tracked
const TASK_STATUSES = [
  'running',
  'error',
  'merged',
  'created',
  'active',
  'completed',
  'failed',
] as const

@injectable()
export class RuntimeHealthCollector {
  private refreshInterval: ReturnType<typeof setInterval> | null = null
  private readonly log = logger.child({ service: 'runtime-health-collector' })

  constructor(
    @inject(WorkspaceSandboxRepository) private readonly sandboxRepo: WorkspaceSandboxRepository,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(TaskRepository) private readonly taskRepo: TaskRepository,
  ) {}

  /**
   * Start the health collector with specified interval
   * @param intervalMs - Collection interval in milliseconds (default: 30000)
   */
  start(intervalMs = 30000): void {
    this.log.info({ intervalMs }, 'Starting runtime health collector')
    this.collectAll() // Initial collection (fire and forget)
    this.refreshInterval = setInterval(() => this.collectAll(), intervalMs)
  }

  /**
   * Stop the health collector
   */
  stop(): void {
    if (this.refreshInterval) {
      clearInterval(this.refreshInterval)
      this.refreshInterval = null
    }
    this.log.info('Stopped runtime health collector')
  }

  /**
   * Collect all health metrics
   * Runs workspace and task collection in parallel.
   * Uses sandbox records as the sole source.
   */
  async collectAll(): Promise<void> {
    try {
      const sandboxRecords = await this.sandboxRepo.list()
      this.log.debug({ sandboxCount: sandboxRecords.length }, 'Collecting health metrics')

      await Promise.all([this.collectWorkspaceHealth(sandboxRecords), this.collectTaskHealth()])
    } catch (error) {
      this.log.error({ err: error }, 'Failed to collect health metrics')
    }
  }

  /**
   * Collect workspace/pod health metrics
   * Uses getValidatedRecord() to query actual K8s pod phase
   */
  private async collectWorkspaceHealth(records: SandboxRecord[]): Promise<void> {
    const startTime = Date.now()
    try {
      // Reset all labels to 0
      for (const state of DOCKER_STATES) {
        workspaceHealth.labels({ mode: 'docker', state }).set(0)
      }
      for (const phase of K8S_PHASES) {
        workspaceHealth.labels({ mode: 'kubernetes', state: phase }).set(0)
      }

      // Get live-verified state for each workspace
      for (const record of records) {
        try {
          // Use getValidatedRecord for live container/pod verification
          const validated = await this.runtimeManager.getValidatedRecord(record.taskId)
          if (!validated) {
            continue
          }

          // Use unified status field for both Docker and Kubernetes
          const state = validated.executionTarget.status ?? 'not_created'
          const mode = isDockerExecutionTarget(validated.executionTarget) ? 'docker' : 'kubernetes'
          workspaceHealth.labels({ mode, state }).inc()
        } catch (error) {
          this.log.warn({ taskId: record.taskId, err: error }, 'Failed to verify workspace health')
        }
      }
      healthCollectorDuration.labels({ type: 'workspace' }).observe((Date.now() - startTime) / 1000)
    } catch (error) {
      healthCollectorErrors.labels({ type: 'workspace' }).inc()
      this.log.error({ err: error }, 'Failed to collect workspace health')
    }
  }

  /**
   * Collect task health metrics
   * Queries all tasks from DB and counts by status
   * - Closed tasks are excluded
   * - Open tasks show their iteration status (created, active, completed, failed)
   * - Other tasks show their task status (running, error, merged)
   */
  private async collectTaskHealth(): Promise<void> {
    const startTime = Date.now()
    try {
      // Reset all labels to 0
      for (const status of TASK_STATUSES) {
        taskHealth.labels({ status }).set(0)
      }

      // Get all tasks (internal, no access control)
      const tasks = await this.taskRepo.listAll()

      for (const task of tasks) {
        // Skip closed tasks
        if (task.status === 'closed') {
          continue
        }

        taskHealth.labels({ status: task.status }).inc()
      }

      healthCollectorDuration.labels({ type: 'task' }).observe((Date.now() - startTime) / 1000)
    } catch (error) {
      healthCollectorErrors.labels({ type: 'task' }).inc()
      this.log.error({ err: error }, 'Failed to collect task health')
    }
  }
}
