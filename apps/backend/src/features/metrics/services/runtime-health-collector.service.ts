/**
 * RuntimeHealthCollector Service
 *
 * Periodically collects live-verified health metrics for workspaces, preview servers, and agents.
 * Uses actual API calls to verify resource state rather than just reading database state.
 *
 * Responsibilities:
 * - Query workspace snapshots from database
 * - Verify K8s pod phase via getValidatedSnapshot() (queries K8s API)
 * - Verify preview server status via PreviewOrchestratorService.getStatus()
 * - Verify agent state via isAgentActuallyRunning() (queries worker endpoints)
 * - Update Prometheus gauges with verified state distribution
 */

import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { PreviewOrchestratorService } from '../../preview/services/preview-orchestrator.service'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeRepository } from '../../workspace/db/workspace-runtime.repository'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import {
  isDockerExecutionTarget,
  type WorkspaceRuntimeSnapshot,
} from '../../workspace/types/workspace-runtime.types'
import {
  agentHealth,
  healthCollectorDuration,
  healthCollectorErrors,
  previewHealth,
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
const PREVIEW_STATUSES = ['stopped', 'installing', 'starting', 'started', 'failed'] as const
const AGENT_STATES = ['idle', 'running'] as const
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
    @inject(WorkspaceRuntimeRepository) private readonly workspaceRepo: WorkspaceRuntimeRepository,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(PreviewOrchestratorService)
    private readonly previewOrchestrator: PreviewOrchestratorService,
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
   * Runs workspace, preview, and agent collection in parallel
   */
  async collectAll(): Promise<void> {
    try {
      const snapshots = await this.workspaceRepo.list()
      this.log.debug({ count: snapshots.length }, 'Collecting health metrics')

      await Promise.all([
        this.collectWorkspaceHealth(snapshots),
        this.collectPreviewHealth(snapshots),
        this.collectAgentHealth(snapshots),
        this.collectTaskHealth(),
      ])
    } catch (error) {
      this.log.error({ err: error }, 'Failed to collect health metrics')
    }
  }

  /**
   * Collect workspace/pod health metrics
   * Uses getValidatedSnapshot() to query actual K8s pod phase
   */
  private async collectWorkspaceHealth(snapshots: WorkspaceRuntimeSnapshot[]): Promise<void> {
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
      for (const snapshot of snapshots) {
        try {
          // Use getValidatedSnapshot for live container/pod verification
          const validated = await this.runtimeManager.getValidatedSnapshot(snapshot.taskId)
          if (!validated) {
            continue
          }

          // Use unified status field for both Docker and Kubernetes
          const state = validated.executionTarget.status ?? 'not_created'
          const mode = isDockerExecutionTarget(validated.executionTarget) ? 'docker' : 'kubernetes'
          workspaceHealth.labels({ mode, state }).inc()
        } catch (error) {
          this.log.warn(
            { taskId: snapshot.taskId, err: error },
            'Failed to verify workspace health',
          )
        }
      }
      healthCollectorDuration.labels({ type: 'workspace' }).observe((Date.now() - startTime) / 1000)
    } catch (error) {
      healthCollectorErrors.labels({ type: 'workspace' }).inc()
      this.log.error({ err: error }, 'Failed to collect workspace health')
    }
  }

  /**
   * Collect preview server health metrics
   * Uses PreviewOrchestratorService.getStatus() for live server verification
   */
  private async collectPreviewHealth(snapshots: WorkspaceRuntimeSnapshot[]): Promise<void> {
    const startTime = Date.now()
    try {
      // Reset all labels to 0
      for (const status of PREVIEW_STATUSES) {
        previewHealth.labels({ status }).set(0)
      }

      // Get live preview server status for each workspace
      for (const snapshot of snapshots) {
        try {
          const status = await this.previewOrchestrator.getStatus(snapshot.taskId)
          previewHealth.labels({ status: status.status }).inc()
        } catch (error) {
          // If we can't get status, count as stopped
          previewHealth.labels({ status: 'stopped' }).inc()
          this.log.debug(
            { taskId: snapshot.taskId, err: error },
            'Failed to get preview status, counting as stopped',
          )
        }
      }
      healthCollectorDuration.labels({ type: 'preview' }).observe((Date.now() - startTime) / 1000)
    } catch (error) {
      healthCollectorErrors.labels({ type: 'preview' }).inc()
      this.log.error({ err: error }, 'Failed to collect preview health')
    }
  }

  /**
   * Collect agent health metrics
   * Uses isAgentActuallyRunning() to verify agent state via worker endpoint
   */
  private async collectAgentHealth(snapshots: WorkspaceRuntimeSnapshot[]): Promise<void> {
    const startTime = Date.now()
    try {
      // Reset all labels to 0
      for (const state of AGENT_STATES) {
        agentHealth.labels({ state }).set(0)
      }

      // Verify agent state for each workspace
      for (const snapshot of snapshots) {
        try {
          const isRunning = await this.runtimeManager.isAgentActuallyRunning(snapshot.taskId)
          agentHealth.labels({ state: isRunning ? 'running' : 'idle' }).inc()
        } catch (error) {
          // If we can't verify, count as idle
          agentHealth.labels({ state: 'idle' }).inc()
          this.log.debug(
            { taskId: snapshot.taskId, err: error },
            'Failed to verify agent state, counting as idle',
          )
        }
      }
      healthCollectorDuration.labels({ type: 'agent' }).observe((Date.now() - startTime) / 1000)
    } catch (error) {
      healthCollectorErrors.labels({ type: 'agent' }).inc()
      this.log.error({ err: error }, 'Failed to collect agent health')
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

        // For open tasks, use the iteration status
        if (task.status === 'open') {
          const lastIteration = task.iterations[task.iterations.length - 1]
          if (lastIteration) {
            taskHealth.labels({ status: lastIteration.status }).inc()
          }
        } else {
          // For running, error, merged - use task status
          taskHealth.labels({ status: task.status }).inc()
        }
      }

      healthCollectorDuration.labels({ type: 'task' }).observe((Date.now() - startTime) / 1000)
    } catch (error) {
      healthCollectorErrors.labels({ type: 'task' }).inc()
      this.log.error({ err: error }, 'Failed to collect task health')
    }
  }
}
