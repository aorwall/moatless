import { randomUUID } from 'node:crypto'
import type {
  AgentEvent,
  Event,
  PendingResponsePayload,
  SDKResultMessage,
  ServerEvent,
  Task,
  TaskEvent,
  TaskIterationEvent,
} from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/index'
import {
  agentCostUsdTotal,
  agentInputTokensTotal,
  agentOutputTokensTotal,
  taskIterationDurationSeconds,
  taskIterationTotal,
} from '../../metrics'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../db/task.repository'
import { TaskExecutionCoordinatorSymbol } from '../symbols'
import { PendingResponseService } from './pending-response.service'
import type { TaskExecutionCoordinator } from './task-execution-coordinator.service'

/**
 * TaskEventHandler - Centralized event subscription and handling for task-related events
 *
 * Handles:
 * - Agent events (started, message, completed, error)
 * - Environment events (ready) for auto-resume
 * - Iteration recovery on startup
 * - Iteration cleanup on shutdown
 *
 * This class consolidates event handling that was previously split between
 * TaskIterationService and TaskExecutionCoordinator into a single subscription.
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskEventHandler {
  private unsubscribe?: (() => void) | undefined

  constructor(
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(EventService) private readonly eventService: EventService,
    @inject(TaskExecutionCoordinatorSymbol)
    private readonly executionCoordinator: TaskExecutionCoordinator,
    @inject(PendingResponseService) private readonly pendingResponseService: PendingResponseService,
  ) {}

  /**
   * Start listening for task-related events from EventService.
   * Called once during app initialization.
   *
   * Handles:
   * - agent.started: Updates agent runtime state to 'running'
   * - agent.message: Captures agentSessionId and contextTokens
   * - agent.completed: Marks iteration as completed/failed
   * - agent.error: Marks iteration as failed
   * - environment.ready: Auto-resumes unfinished iterations
   */
  startEventSubscription(): void {
    if (this.unsubscribe) {
      logger.warn('TaskEventHandler: Event subscription already started')
      return
    }

    this.unsubscribe = this.eventService.subscribeAll(async (event) => {
      try {
        if (event.type === 'agent') {
          await this.handleAgentEvent(event as AgentEvent)
        } else if (event.type === 'environment' && event.subType === 'ready') {
          await this.handleEnvironmentReadyEvent(event)
        } else if (event.type === 'server') {
          await this.handleServerEvent(event as ServerEvent)
        }
      } catch (error) {
        logger.warn(
          {
            taskId: event.taskId,
            eventId: event.eventId,
            type: event.type,
            subType: event.subType,
            error: error instanceof Error ? error.message : String(error),
          },
          'Failed to handle event',
        )
      }
    })
    logger.info('TaskEventHandler: Started event subscription')
  }

  /**
   * Stop event subscription (for graceful shutdown)
   */
  stopEventSubscription(): void {
    if (this.unsubscribe) {
      this.unsubscribe()
      this.unsubscribe = undefined
      logger.info('TaskEventHandler: Stopped event subscription')
    }
  }

  /**
   * Route agent events to appropriate handlers
   */
  private async handleAgentEvent(event: AgentEvent): Promise<void> {
    switch (event.subType) {
      case 'started':
        await this.handleAgentStartedEvent(event)
        break
      case 'message':
        await this.handleAgentMessageEvent(event)
        break
      case 'completed':
        // Clear any pending response state when iteration completes
        this.pendingResponseService.clearPending(event.taskId)
        await this.handleAgentCompletedEvent(event)
        break
      case 'error':
        // Clear any pending response state when iteration errors
        this.pendingResponseService.clearPending(event.taskId)
        await this.handleAgentErrorEvent(event)
        break
      case 'pending_response':
        this.handlePendingResponseEvent(event)
        break
    }
  }

  /**
   * Handle agent.pending_response event by tracking the pending interactive tool request.
   * This allows channels (Slack, Email) to detect when a task is waiting for user input.
   */
  private handlePendingResponseEvent(event: AgentEvent): void {
    const taskLogger = logger.child({ taskId: event.taskId, eventId: event.eventId })
    const payload = event.payload as PendingResponsePayload

    if (!payload.toolName) {
      taskLogger.warn('pending_response event missing toolName')
      return
    }

    this.pendingResponseService.setPending(event.taskId, payload.toolName, payload.toolInput || {})

    taskLogger.info({ toolName: payload.toolName }, 'Tracked pending interactive tool request')
  }

  /**
   * Handle agent.started event by updating agent runtime state.
   */
  private async handleAgentStartedEvent(event: AgentEvent): Promise<void> {
    const taskLogger = logger.child({ taskId: event.taskId, eventId: event.eventId })
    taskLogger.debug('Handling agent.started event')

    try {
      await this.runtimeManager.updateAgentState(event.taskId, {
        state: 'running',
        agentType: event.agentType,
        startedAt: new Date(),
      })
      this.runtimeManager.touchActivity(event.taskId)
    } catch (error) {
      taskLogger.warn(
        { error: error instanceof Error ? error.message : String(error) },
        'Failed to update agent state on started event',
      )
    }
  }

  /**
   * Handle agent.message event by capturing agentSessionId and contextTokens.
   * Persists session ID to iteration for resume support.
   */
  private async handleAgentMessageEvent(event: AgentEvent): Promise<void> {
    const taskId = event.taskId
    this.runtimeManager.touchActivity(taskId)
    const payload = event.payload

    // Extract agentSessionId if present
    const agentSessionId =
      payload && typeof payload === 'object' && 'agentSessionId' in payload
        ? (payload.agentSessionId as string)
        : undefined

    // Extract contextTokens if present
    const contextTokens =
      payload && typeof payload === 'object' && 'contextTokens' in payload
        ? (payload.contextTokens as number)
        : undefined

    if (!agentSessionId && contextTokens === undefined) {
      return // Nothing to update
    }

    const taskLogger = logger.child({ taskId, eventId: event.eventId })

    // Get the task
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      return
    }

    // Find active iteration
    const activeIteration = task.iterations.find((it: { status: string }) => it.status === 'active')
    if (!activeIteration) {
      return
    }

    let needsSave = false

    // Capture agentSessionId
    if (agentSessionId && activeIteration.agentSessionId !== agentSessionId) {
      taskLogger.info(
        { previousSessionId: activeIteration.agentSessionId, newSessionId: agentSessionId },
        'Capturing agent session ID from message event',
      )
      activeIteration.agentSessionId = agentSessionId
      needsSave = true

      // Update runtime state with session ID
      try {
        await this.runtimeManager.updateAgentState(taskId, { sessionId: agentSessionId })
      } catch (error) {
        taskLogger.warn(
          { error: error instanceof Error ? error.message : String(error) },
          'Failed to update runtime state with session ID',
        )
      }
    }

    // Track context tokens
    if (contextTokens !== undefined) {
      activeIteration.contextTokens = contextTokens
      needsSave = true
    }

    if (needsSave) {
      await this.taskRepository.save(task as Task)
    }
  }

  /**
   * Handle agent.completed event by marking the iteration as completed/failed.
   *
   * Note: Uses getInternal() to bypass access control since this runs in a background
   * event handler context without user session.
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Event handler with multiple conditional branches for payload processing
  private async handleAgentCompletedEvent(event: AgentEvent): Promise<void> {
    const taskId = event.taskId
    const payload = event.payload as SDKResultMessage

    const taskLogger = logger.child({ taskId, eventId: event.eventId })

    taskLogger.info(
      {
        isError: payload.is_error,
        subtype: payload.subtype,
        costUsd: payload.total_cost_usd,
      },
      'Handling agent.completed event for iteration completion',
    )

    // Get the task - use getInternal to bypass access control since this runs
    // in a background event handler without user session context
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      taskLogger.warn('Task not found for agent.completed event')
      return
    }

    // Find active iteration
    const activeIteration = task.iterations.find((it: { status: string }) => it.status === 'active')
    if (!activeIteration) {
      taskLogger.warn('No active iteration found for agent.completed event')
      return
    }

    const previousTaskStatus = task.status

    // Apply result to iteration based on payload
    if (payload.is_error) {
      activeIteration.status = 'failed'
      // Extract error message based on subtype
      let errorMessage = 'Agent execution failed'
      if (payload.subtype === 'error_max_turns') {
        errorMessage = 'Maximum number of turns exceeded'
      } else if (payload.subtype === 'error_during_execution') {
        // Extract actual errors from SDK payload
        const errors =
          'errors' in payload && Array.isArray(payload.errors) ? (payload.errors as string[]) : []
        errorMessage = errors.length > 0 ? errors.join('; ') : 'Error occurred during execution'
      }
      activeIteration.result =
        payload.subtype === 'success' && payload.result ? payload.result : errorMessage
    } else {
      activeIteration.status = 'completed'
      // Only success subtype has result field
      activeIteration.result =
        payload.subtype === 'success' && payload.result
          ? payload.result
          : 'Agent completed successfully'
    }

    // Update cost and token metrics from payload
    if (payload.total_cost_usd !== undefined) {
      activeIteration.costUsd = payload.total_cost_usd
    }

    // Update token usage from payload
    const usage = payload.usage as { input_tokens?: number; output_tokens?: number } | undefined
    if (usage?.input_tokens !== undefined) {
      activeIteration.inputTokens = usage.input_tokens
    }
    if (usage?.output_tokens !== undefined) {
      activeIteration.outputTokens = usage.output_tokens
    }

    activeIteration.endTime = new Date()

    // Track Prometheus metrics with delta support for resumed sessions
    const agentType = event.agentType || 'unknown'
    const costDelta = (payload.total_cost_usd ?? 0) - (activeIteration.baselineCostUsd ?? 0)
    const inputTokensDelta = (usage?.input_tokens ?? 0) - (activeIteration.baselineInputTokens ?? 0)
    const outputTokensDelta =
      (usage?.output_tokens ?? 0) - (activeIteration.baselineOutputTokens ?? 0)

    if (costDelta > 0) {
      agentCostUsdTotal.labels({ agent_type: agentType }).inc(costDelta)
    }
    if (inputTokensDelta > 0) {
      agentInputTokensTotal.labels({ agent_type: agentType }).inc(inputTokensDelta)
    }
    if (outputTokensDelta > 0) {
      agentOutputTokensTotal.labels({ agent_type: agentType }).inc(outputTokensDelta)
    }

    // Track iteration completion metrics
    taskIterationTotal.labels({ status: activeIteration.status, agent_type: agentType }).inc()
    if (activeIteration.startTime) {
      const durationSeconds =
        (activeIteration.endTime.getTime() - activeIteration.startTime.getTime()) / 1000
      taskIterationDurationSeconds.labels({ agent_type: agentType }).observe(durationSeconds)
    }

    // Update task status based on iteration outcome
    task.status = activeIteration.status === 'failed' ? 'error' : 'open'
    task.lastMessageAt = new Date()

    // Calculate total cost from all iterations
    task.totalCostUsd = task.iterations.reduce(
      (sum: number, it: { costUsd?: number }) => sum + (it.costUsd || 0),
      0,
    )

    // Save task with updated iteration
    await this.taskRepository.save(task as Task)

    // Reset agent state - agent is done
    try {
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
        sessionId: undefined,
        startedAt: undefined,
      })
    } catch (stateError) {
      taskLogger.warn(
        { error: stateError instanceof Error ? stateError.message : String(stateError) },
        'Failed to reset agent state after completion',
      )
    }

    // Emit iteration completion event
    const iterationEventSubType = activeIteration.status === 'completed' ? 'completed' : 'error'
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'taskIteration',
      subType: iterationEventSubType,
      agentType: task.agentType,
      iterationId: activeIteration.iterationNumber,
      payload: {
        status: activeIteration.status,
        previousStatus: 'active',
        iterationNumber: activeIteration.iterationNumber,
        result: activeIteration.result,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: activeIteration.status === 'failed',
    } as TaskIterationEvent)

    // Emit task status change event if status changed
    if (previousTaskStatus !== task.status) {
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'task',
        subType: task.status as 'open' | 'closed',
        agentType: task.agentType,
        payload: {
          status: task.status,
          previousStatus: previousTaskStatus,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        isError: task.status === 'error',
      } as TaskEvent)
    }

    taskLogger.info(
      {
        iterationNumber: activeIteration.iterationNumber,
        status: activeIteration.status,
        taskStatus: task.status,
      },
      'Iteration completed via agent.completed event',
    )
  }

  /**
   * Handle agent.error event by marking the iteration as failed.
   * This handles errors that occur during agent execution in the sandbox.
   */
  private async handleAgentErrorEvent(event: AgentEvent): Promise<void> {
    const taskId = event.taskId
    const payload = event.payload as { error?: string }

    const taskLogger = logger.child({ taskId, eventId: event.eventId })

    taskLogger.info({ error: payload?.error }, 'Handling agent.error event for iteration failure')

    // Get the task
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      taskLogger.warn('Task not found for agent.error event')
      return
    }

    // Find active iteration
    const activeIteration = task.iterations.find((it: { status: string }) => it.status === 'active')
    if (!activeIteration) {
      taskLogger.warn('No active iteration found for agent.error event')
      return
    }

    const previousTaskStatus = task.status

    // Apply error to iteration
    activeIteration.status = 'failed'
    activeIteration.result = payload?.error || 'Agent execution failed'
    activeIteration.endTime = new Date()

    // Update task status
    task.status = 'error'
    task.lastMessageAt = new Date()

    // Save task
    await this.taskRepository.save(task as Task)

    // Reset agent state - agent errored
    try {
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
        sessionId: undefined,
        startedAt: undefined,
      })
    } catch (stateError) {
      taskLogger.warn(
        { error: stateError instanceof Error ? stateError.message : String(stateError) },
        'Failed to reset agent state after error',
      )
    }

    // Emit iteration error event
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'taskIteration',
      subType: 'error',
      agentType: task.agentType,
      iterationId: activeIteration.iterationNumber,
      payload: {
        status: 'failed',
        previousStatus: 'active',
        iterationNumber: activeIteration.iterationNumber,
        result: activeIteration.result,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: true,
    } as TaskIterationEvent)

    // Emit task status change event if status changed
    if (previousTaskStatus !== task.status) {
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'task',
        subType: task.status as 'open' | 'closed',
        agentType: task.agentType,
        payload: {
          status: task.status,
          previousStatus: previousTaskStatus,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        isError: true,
      } as TaskEvent)
    }

    taskLogger.info(
      {
        iterationNumber: activeIteration.iterationNumber,
        taskStatus: task.status,
      },
      'Iteration failed via agent.error event',
    )
  }

  /**
   * Handle environment:ready event by checking for unfinished iterations and auto-resuming.
   * This enables automatic recovery of task execution after workspace restart.
   *
   * Behavior:
   * - 'active' iteration WITH agentSessionId: Resume using SDK session resume feature
   * - 'active' iteration WITHOUT agentSessionId: Start fresh using initialMessage
   * - 'created' iteration: Start the iteration normally
   */
  private async handleEnvironmentReadyEvent(event: Event): Promise<void> {
    const taskId = event.taskId
    const taskLogger = logger.child({ taskId, handler: 'handleEnvironmentReadyEvent' })

    try {
      // Use getInternal to bypass access control since this runs in background
      const task = await this.taskRepository.getInternal(taskId)
      if (!task) {
        taskLogger.debug('Task not found for environment:ready event')
        return
      }

      // Find unfinished iteration (active or created)
      const unfinishedIteration = task.iterations.find(
        (it) => it.status === 'active' || it.status === 'created',
      )

      if (!unfinishedIteration) {
        taskLogger.debug('No unfinished iteration to resume')
        return
      }

      taskLogger.info(
        {
          iterationNumber: unfinishedIteration.iterationNumber,
          iterationStatus: unfinishedIteration.status,
          hasAgentSessionId: !!unfinishedIteration.agentSessionId,
        },
        'Auto-resuming unfinished iteration after workspace ready',
      )

      if (unfinishedIteration.status === 'active') {
        // Resume the active iteration
        await this.executionCoordinator.resumeIteration(taskId, unfinishedIteration)
      } else if (unfinishedIteration.status === 'created') {
        // Start the created iteration
        await this.executionCoordinator.startTask(taskId)
      }
    } catch (error) {
      taskLogger.error(
        { error: error instanceof Error ? error.message : String(error) },
        'Failed to auto-resume iteration after workspace ready',
      )
    }
  }

  /**
   * Handle server events (preview server status changes from sandbox).
   * Updates the workspace snapshot's preview state when sandbox emits status changes.
   */
  private async handleServerEvent(event: ServerEvent): Promise<void> {
    const taskId = event.taskId
    const taskLogger = logger.child({ taskId, handler: 'handleServerEvent' })

    // Extract status from event - subType is 'installing' | 'starting' | 'started' | 'stopped' | 'failed' | 'error'
    // Map 'error' to 'failed' for preview state compatibility
    const eventStatus = event.subType
    const status: 'stopped' | 'installing' | 'starting' | 'started' | 'failed' =
      eventStatus === 'error' ? 'failed' : eventStatus
    const payload = event.payload

    taskLogger.debug(
      { status, eventStatus, port: payload?.port, error: payload?.error },
      'Handling server status change event',
    )

    try {
      // Update preview state in workspace snapshot
      await this.runtimeManager.updatePreviewState(taskId, {
        status,
        port: payload?.port,
        lastError: payload?.error,
        ...(status === 'started' && { lastStartedAt: new Date() }),
        ...(status === 'stopped' && { lastStoppedAt: new Date() }),
      })

      taskLogger.debug({ status }, 'Updated preview state from server event')
    } catch (error) {
      taskLogger.warn(
        { error: error instanceof Error ? error.message : String(error) },
        'Failed to update preview state from server event',
      )
    }
  }

  /**
   * Recover orphaned active iterations from previous server runs.
   * Called during application startup to clean up iterations that were
   * left in 'active' or 'created' state due to server crash or unexpected shutdown.
   *
   * Smart recovery behavior:
   * - If workspace is 'initializing' or 'ready': Leave iteration for auto-resume
   *   (handleEnvironmentReadyEvent will handle via environment:ready event)
   * - If workspace is error/stopped/not_created: Mark active iterations as failed
   * - 'created' iterations without workspace can stay (user can manually start later)
   *
   * @returns Number of iterations marked as failed
   */
  async recoverOrphanedIterations(): Promise<number> {
    try {
      logger.info('Checking for orphaned iterations...')

      // Get all tasks directly from repository (bypasses access control)
      const allTasks = await this.taskRepository.listAll()

      let recoveredCount = 0
      let queuedForAutoResume = 0

      for (const task of allTasks) {
        // Find unfinished iteration (active or created)
        const unfinishedIteration = task.iterations.find(
          (it: { status: string }) => it.status === 'active' || it.status === 'created',
        )

        if (!unfinishedIteration) {
          continue
        }

        // Check workspace status
        const snapshot = await this.runtimeManager.getSnapshot(task.taskId)
        const workspaceStatus = snapshot?.executionTarget?.status

        const iterationLogger = logger.child({
          taskId: task.taskId,
          iterationStatus: unfinishedIteration.status,
          workspaceStatus,
          hasAgentSessionId: !!unfinishedIteration.agentSessionId,
        })

        // If workspace is initializing or ready, let auto-resume handle it
        if (workspaceStatus === 'initializing' || workspaceStatus === 'ready') {
          iterationLogger.info('Iteration queued for auto-resume (workspace available)')
          queuedForAutoResume++
          continue
        }

        // Only mark 'active' iterations as failed when workspace is unavailable
        // 'created' iterations can stay - user can manually start them later
        if (unfinishedIteration.status === 'active') {
          const marked = await this.markIterationAsFailed(
            task.taskId,
            'Recovered from server restart',
            task as Task,
          )
          if (marked) {
            recoveredCount++
            iterationLogger.info('Marked orphaned active iteration as failed')
          }
        } else {
          iterationLogger.debug(
            'Created iteration left in place (no workspace, awaiting manual start)',
          )
        }
      }

      if (recoveredCount > 0 || queuedForAutoResume > 0) {
        logger.info({ recoveredCount, queuedForAutoResume }, 'Orphaned iteration recovery complete')
      } else {
        logger.info('No orphaned iterations found')
      }

      return recoveredCount
    } catch (error) {
      logger.error({ err: error }, 'Error recovering orphaned iterations during startup')
      return 0
    }
  }

  /**
   * Clean up active iterations during graceful server shutdown.
   * Marks all active iterations as failed before the server stops.
   *
   * @returns Number of iterations marked as failed
   */
  async cleanupActiveIterationsOnShutdown(): Promise<number> {
    try {
      logger.info('Marking active iterations as failed...')

      // Get all tasks directly from repository (bypasses access control)
      const allTasks = await this.taskRepository.listAll()

      let markedCount = 0
      for (const task of allTasks) {
        // Check if task has any active iteration
        const hasActiveIteration = task.iterations.some(
          (it: { status: string }) => it.status === 'active',
        )
        if (hasActiveIteration) {
          // Pass task directly to bypass visibility-scoped fetch
          const marked = await this.markIterationAsFailed(
            task.taskId,
            'Server shutdown',
            task as Task,
          )
          if (marked) {
            markedCount++
          }
        }
      }

      if (markedCount > 0) {
        logger.info(`Marked ${markedCount} active iterations as failed`)
      } else {
        logger.info('No active iterations to mark as failed')
      }

      return markedCount
    } catch (error) {
      logger.error({ err: error }, 'Error marking iterations as failed during shutdown')
      return 0
    }
  }

  /**
   * Mark an active iteration as failed with a specific reason.
   * Used for system-level failures like server shutdown/restart.
   *
   * @param taskId - Task ID
   * @param reason - Reason for failure (e.g., 'Server shutdown', 'Recovered from server restart')
   * @param existingTask - Optional task object (bypasses visibility-scoped fetch for system operations)
   * @returns True if an active iteration was found and marked as failed
   */
  private async markIterationAsFailed(
    taskId: string,
    reason: string,
    existingTask?: Task,
  ): Promise<boolean> {
    const taskLogger = logger.child({ taskId })

    try {
      // Use provided task or fetch it (fetch goes through visibility scoping)
      const task = existingTask ?? (await this.taskRepository.get(taskId))
      if (!task) {
        taskLogger.debug('Task not found when marking iteration as failed')
        return false
      }

      // Find the active iteration
      const activeIteration = task.iterations.find((it) => it.status === 'active')
      if (!activeIteration) {
        taskLogger.debug('No active iteration found to mark as failed')
        return false
      }

      // Mark iteration as failed
      activeIteration.status = 'failed'
      activeIteration.result = reason
      activeIteration.endTime = new Date()

      taskLogger.info(
        {
          iterationNumber: activeIteration.iterationNumber,
          reason,
        },
        'Marked iteration as failed',
      )

      // Save the task
      await this.taskRepository.save(task)

      // Emit iteration error event
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'taskIteration',
        subType: 'error',
        agentType: task.agentType,
        iterationId: activeIteration.iterationNumber,
        payload: {
          status: 'failed',
          previousStatus: 'active',
          iterationNumber: activeIteration.iterationNumber,
          result: reason,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        isError: true,
      } as TaskIterationEvent)

      return true
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to mark iteration as failed')
      return false
    }
  }
}
