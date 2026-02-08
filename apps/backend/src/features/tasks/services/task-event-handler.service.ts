import { randomUUID } from 'node:crypto'
import type {
  AgentEvent,
  ResultAgentMessage,
  SDKResultMessage,
  ServerEvent,
  Task,
  TaskEvent,
  TaskTurnEvent,
} from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/index'
import {
  agentCostUsdTotal,
  agentInputTokensTotal,
  agentOutputTokensTotal,
  taskTurnTotal,
} from '../../metrics'
import { TagRepository } from '../../tags/db/tag.repository'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../db/task.repository'
import { TaskMessageRepository } from '../db/task-message.repository'
import { TaskService } from './task.service'

/**
 * TaskEventHandler - Centralized event subscription and handling for task-related events
 *
 * Handles:
 * - Agent events (started, message, completed, error, stopped)
 * - Server events (preview server status changes)
 * - Task metadata events (from sandbox filesystem monitor)
 * - Task recovery on startup (finds tasks with status='running', marks as error if workspace unavailable)
 * - Task cleanup on shutdown (marks all running tasks as 'error')
 *
 * Note: Auto-resume on environment:ready was removed because the queue-based architecture
 * handles recovery naturally - unacknowledged messages remain on the queue and are
 * picked up when the workspace restarts.
 *
 * This class consolidates all event handling into a single subscription.
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskEventHandler {
  private unsubscribe?: (() => void) | undefined

  constructor(
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(TaskMessageRepository) private readonly messageRepository: TaskMessageRepository,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(EventService) private readonly eventService: EventService,
    @inject(TaskService) private readonly taskService: TaskService,
    @inject(TagRepository) private readonly tagRepository: TagRepository,
  ) {}

  /**
   * Start listening for task-related events from EventService.
   * Called once during app initialization.
   *
   * Handles:
   * - agent.started: Updates agent runtime state to 'running'
   * - agent.message: Captures agentSessionId in runtime state
   * - agent.completed: Appends result message, updates task status and cost
   * - agent.error: Appends error result message, marks task as error
   * - agent.stopped: Appends stopped result message, marks task as open
   * - server.*: Updates preview state in workspace snapshot
   * - task.metadataUpdated: Updates task name/description from sandbox
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
        } else if (event.type === 'server') {
          await this.handleServerEvent(event as ServerEvent)
        } else if (event.type === 'task' && (event.subType as string) === 'metadataUpdated') {
          await this.handleTaskMetadataUpdatedEvent(event as TaskEvent)
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
      case 'stopped':
        await this.handleAgentStoppedEvent(event)
        break
      case 'message':
        await this.handleAgentMessageEvent(event)
        break
      case 'completed':
        await this.handleAgentCompletedEvent(event)
        break
      case 'error':
        await this.handleAgentErrorEvent(event)
        break
      case 'pending_response':
        // No longer tracking pending state — sandbox handles routing internally
        break
    }
  }

  /**
   * Handle agent.started event by updating agent runtime state.
   */
  private handleAgentStartedEvent(event: AgentEvent): void {
    const taskLogger = logger.child({ taskId: event.taskId, eventId: event.eventId })
    taskLogger.debug('Handling agent.started event')
    // Agent state is ephemeral — sandbox tracks it.
  }

  /**
   * Handle agent.stopped event when agent is stopped by user request.
   * Appends a result message with turnStatus='completed' and turnResult='Stopped by user request'.
   * Updates task status to 'open'.
   */
  private async handleAgentStoppedEvent(event: AgentEvent): Promise<void> {
    const taskId = event.taskId
    const taskLogger = logger.child({ taskId, eventId: event.eventId })

    taskLogger.info('Handling agent.stopped event - agent stopped by user request')

    // Agent state is ephemeral — sandbox tracks it internally.

    // Get the task
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      taskLogger.warn('Task not found for agent.stopped event')
      return
    }

    const previousTaskStatus = task.status
    const turnNumber = 0

    // Agent session ID is ephemeral — not persisted
    const agentSessionId = undefined

    // Build result message
    const resultMessage: ResultAgentMessage = {
      uuid: randomUUID(),
      type: 'result',
      userType: 'system',
      turnNumber,
      timestamp: new Date().toISOString(),
      turnStatus: 'completed',
      turnResult: 'Stopped by user request',
      agentSessionId,
      willNotResume: false,
      runMetrics: {
        costUsd: 0,
        inputTokens: 0,
        outputTokens: 0,
        contextTokens: 0,
      },
    }

    // Append result message
    await this.messageRepository.append(taskId, resultMessage)

    // Update task status to 'open'
    task.status = 'open'
    task.lastMessageAt = new Date()

    // Save the task
    await this.taskRepository.save(task as Task)

    // Emit taskTurn completion event
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: 'taskTurn',
      subType: 'completed',
      agentType: task.agentType,
      turnNumber,
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: {
        status: 'open',
        previousStatus: previousTaskStatus,
        turnNumber,
      },
    } as TaskTurnEvent)

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
        isError: false,
      } as TaskEvent)
    }

    taskLogger.info(
      { turnNumber, taskStatus: task.status },
      'Turn marked as stopped by user request',
    )
  }

  /**
   * Handle agent.message event by capturing agentSessionId in runtime state
   * and auto-tagging tasks when the Skill tool is invoked.
   */
  private handleAgentMessageEvent(event: AgentEvent): void {
    const taskId = event.taskId
    const payload = event.payload

    // Auto-tag task when Skill tool is used (fire-and-forget, non-blocking)
    this.extractSkillTags(taskId, payload)

    // Agent session ID is ephemeral — sandbox tracks it internally.
    // No need to persist in DB.
  }

  /**
   * Extract skill names from Skill tool_use blocks in agent messages
   * and auto-tag the task with `skill-{name}`.
   */
  private extractSkillTags(taskId: string, payload: Record<string, unknown> | unknown): void {
    try {
      const p = payload as { message?: { content?: unknown[] } }
      const content = p?.message?.content
      if (!Array.isArray(content)) {
        return
      }

      for (const block of content) {
        const b = block as { type?: string; name?: string; input?: { skill?: string } }
        if (b.type === 'tool_use' && b.name === 'Skill' && typeof b.input?.skill === 'string') {
          const skillName = b.input.skill
          const tagName = `skill-${skillName}`

          // Fire-and-forget: tag the task in the background
          this.tagRepository
            .getOrCreateByName(tagName)
            .then((tag) => this.tagRepository.addTagToTask(taskId, tag.tagId))
            .catch((error) => {
              logger.warn(
                { taskId, tagName, error: error instanceof Error ? error.message : String(error) },
                'Failed to auto-tag task with skill',
              )
            })
        }
      }
    } catch {
      // Silently ignore malformed payloads
    }
  }

  /**
   * Handle agent.completed event by appending a result message and updating task status/cost.
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
      'Handling agent.completed event for turn completion',
    )

    // Get the task - use getInternal to bypass access control since this runs
    // in a background event handler without user session context
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      taskLogger.warn('Task not found for agent.completed event')
      return
    }

    const previousTaskStatus = task.status
    const turnNumber = 0

    // Agent session ID is ephemeral — extract from event payload if available
    const agentSessionId =
      event.payload && typeof event.payload === 'object' && 'session_id' in event.payload
        ? (event.payload.session_id as string)
        : undefined

    // Determine turn status and result
    let turnStatus: 'completed' | 'failed' = 'completed'
    let turnResult = 'Agent completed successfully'
    let willNotResume = false

    if (payload.is_error) {
      turnStatus = 'failed'
      willNotResume = true // SDK doesn't support resuming error sessions

      // Extract error message based on subtype
      if (payload.subtype === 'error_max_turns') {
        turnResult = 'Maximum number of turns exceeded'
      } else if (payload.subtype === 'error_during_execution') {
        // Extract actual errors from SDK payload
        const errors =
          'errors' in payload && Array.isArray(payload.errors) ? (payload.errors as string[]) : []
        turnResult = errors.length > 0 ? errors.join('; ') : 'Error occurred during execution'
      } else {
        turnResult = 'Agent execution failed'
      }
    } else {
      // Only success subtype has result field
      turnResult =
        payload.subtype === 'success' && payload.result
          ? payload.result
          : 'Agent completed successfully'
    }

    // Extract metrics from payload
    const costUsd = payload.total_cost_usd ?? 0
    const usage = payload.usage as { input_tokens?: number; output_tokens?: number } | undefined
    const inputTokens = usage?.input_tokens ?? 0
    const outputTokens = usage?.output_tokens ?? 0
    const contextTokens = 0 // Context tokens are no longer tracked per-turn

    // Build result message
    const resultMessage: ResultAgentMessage = {
      uuid: randomUUID(),
      type: 'result',
      userType: 'system',
      turnNumber,
      timestamp: new Date().toISOString(),
      turnStatus,
      turnResult,
      agentSessionId,
      willNotResume,
      runMetrics: {
        costUsd,
        inputTokens,
        outputTokens,
        contextTokens,
      },
    }

    // Append result message
    await this.messageRepository.append(taskId, resultMessage)

    // Update task status
    task.status = turnStatus === 'failed' ? 'error' : 'open'
    task.lastMessageAt = new Date()

    // Update task total cost
    task.totalCostUsd = (task.totalCostUsd ?? 0) + costUsd

    // Save task
    await this.taskRepository.save(task as Task)

    // Agent state is ephemeral — sandbox tracks it internally.

    // Track Prometheus metrics
    const agentType = event.agentType || 'unknown'
    if (costUsd > 0) {
      agentCostUsdTotal.labels({ agent_type: agentType }).inc(costUsd)
    }
    if (inputTokens > 0) {
      agentInputTokensTotal.labels({ agent_type: agentType }).inc(inputTokens)
    }
    if (outputTokens > 0) {
      agentOutputTokensTotal.labels({ agent_type: agentType }).inc(outputTokens)
    }

    // Track turn completion metrics
    taskTurnTotal.labels({ status: turnStatus, agent_type: agentType }).inc()

    // Emit taskTurn completion/error event
    const turnEventSubType = turnStatus === 'completed' ? 'completed' : 'error'
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'taskTurn',
      subType: turnEventSubType,
      agentType: task.agentType,
      turnNumber,
      payload: {
        status: task.status,
        previousStatus: previousTaskStatus,
        turnNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: turnStatus === 'failed',
    } as TaskTurnEvent)

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
        turnNumber,
        turnStatus,
        taskStatus: task.status,
      },
      'Turn completed via agent.completed event',
    )
  }

  /**
   * Handle agent.error event by appending an error result message and marking task as error.
   * This handles errors that occur during agent execution in the sandbox.
   */
  private async handleAgentErrorEvent(event: AgentEvent): Promise<void> {
    const taskId = event.taskId
    const payload = event.payload as { error?: string }

    const taskLogger = logger.child({ taskId, eventId: event.eventId })

    taskLogger.info({ error: payload?.error }, 'Handling agent.error event for turn failure')

    // Get the task
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      taskLogger.warn('Task not found for agent.error event')
      return
    }

    const previousTaskStatus = task.status
    const turnNumber = 0

    // Agent session ID is ephemeral — not persisted
    const agentSessionId = undefined

    // Build error result message
    const resultMessage: ResultAgentMessage = {
      uuid: randomUUID(),
      type: 'result',
      userType: 'system',
      turnNumber,
      timestamp: new Date().toISOString(),
      turnStatus: 'failed',
      turnResult: payload?.error || 'Agent execution failed',
      agentSessionId,
      willNotResume: true, // SDK doesn't support resuming error sessions
      runMetrics: {
        costUsd: 0,
        inputTokens: 0,
        outputTokens: 0,
        contextTokens: 0,
      },
    }

    // Append result message
    await this.messageRepository.append(taskId, resultMessage)

    // Update task status
    task.status = 'error'
    task.lastMessageAt = new Date()

    // Save task
    await this.taskRepository.save(task as Task)

    // Agent state is ephemeral — sandbox tracks it internally.

    // Emit taskTurn error event
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'taskTurn',
      subType: 'error',
      agentType: task.agentType,
      turnNumber,
      payload: {
        status: 'error',
        previousStatus: previousTaskStatus,
        turnNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: true,
    } as TaskTurnEvent)

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
        turnNumber,
        taskStatus: task.status,
      },
      'Turn failed via agent.error event',
    )
  }

  /**
   * Handle server events (preview server status changes from sandbox).
   * Updates the workspace snapshot's preview state when sandbox emits status changes.
   */
  private handleServerEvent(event: ServerEvent): void {
    const taskId = event.taskId
    const taskLogger = logger.child({ taskId, handler: 'handleServerEvent' })

    // Extract status from event - subType includes 'error' and 'timeout' which we map to 'failed'/'stopped'
    const eventStatus = event.subType
    const status: 'stopped' | 'installing' | 'starting' | 'started' | 'failed' | 'timeout' =
      eventStatus === 'error' ? 'failed' : eventStatus
    const payload = event.payload

    taskLogger.debug(
      { status, eventStatus, serverName: payload?.serverName, error: payload?.error },
      'Handling server status change event',
    )

    // Preview state is ephemeral — sandbox tracks it internally.
    // Events are forwarded to frontends via SSE but NOT persisted in DB.
    taskLogger.debug({ status }, 'Server event received (preview state is ephemeral)')
  }

  /**
   * Handle task.metadataUpdated event from sandbox filesystem monitor.
   * Updates task name and description via TaskService.
   */
  private async handleTaskMetadataUpdatedEvent(event: TaskEvent): Promise<void> {
    const taskId = event.taskId
    const payload = event.payload as { name?: string; description?: string }

    const taskLogger = logger.child({ taskId, handler: 'handleTaskMetadataUpdatedEvent' })
    taskLogger.info(
      { name: payload.name, description: payload.description },
      'Handling task.metadataUpdated event',
    )

    try {
      // Use TaskService.updateTaskWithEvent() which handles:
      // - Validation
      // - Vibe repository renaming
      // - Event emission for real-time updates
      // Only include defined values to satisfy exactOptionalPropertyTypes
      const updates: { name?: string; description?: string } = {}
      if (payload.name !== undefined) {
        updates.name = payload.name
      }
      if (payload.description !== undefined) {
        updates.description = payload.description
      }
      await this.taskService.updateTaskWithEvent(taskId, updates)

      taskLogger.info({ name: payload.name }, 'Task metadata updated successfully')
    } catch (error) {
      taskLogger.error(
        { error: error instanceof Error ? error.message : String(error) },
        'Failed to update task metadata from event',
      )
    }
  }

  /**
   * Recover orphaned running tasks from previous server runs.
   * Called during application startup to clean up tasks that were
   * left in 'running' state due to server crash or unexpected shutdown.
   *
   * Smart recovery behavior:
   * - If workspace is 'initializing' or 'ready': Leave task for auto-resume
   *   (queue-based architecture will handle recovery naturally)
   * - If workspace is error/stopped/not_created: Mark task as 'error' and append failure message
   *
   * @returns Number of tasks marked as failed
   */
  async recoverOrphanedTasks(): Promise<number> {
    try {
      logger.info('Checking for orphaned running tasks...')

      // Get all tasks directly from repository (bypasses access control)
      const allTasks = await this.taskRepository.listAll()

      let recoveredCount = 0
      let queuedForAutoResume = 0

      for (const task of allTasks) {
        // Skip tasks that are not in running state
        if (task.status !== 'running') {
          continue
        }

        // Check workspace status via sandbox controller
        const wsStatus = await this.runtimeManager.getWorkspaceStatus(task.taskId)

        const taskLogger = logger.child({
          taskId: task.taskId,
          taskStatus: task.status,
          workspaceStatus: wsStatus.status,
        })

        // If workspace is initializing or ready, let auto-resume handle it
        if (wsStatus.status === 'initializing' || wsStatus.status === 'ready') {
          taskLogger.info('Task queued for auto-resume (workspace available)')
          queuedForAutoResume++
          continue
        }

        // Mark task as failed when workspace is unavailable
        const marked = await this.markTaskAsFailed(
          task.taskId,
          'Recovered from server restart',
          task as Task,
        )
        if (marked) {
          recoveredCount++
          taskLogger.info('Marked orphaned running task as failed')
        }
      }

      if (recoveredCount > 0 || queuedForAutoResume > 0) {
        logger.info({ recoveredCount, queuedForAutoResume }, 'Orphaned task recovery complete')
      } else {
        logger.info('No orphaned running tasks found')
      }

      return recoveredCount
    } catch (error) {
      logger.error({ err: error }, 'Error recovering orphaned tasks during startup')
      return 0
    }
  }

  /**
   * Clean up running tasks during graceful server shutdown.
   * Marks all running tasks as 'error' and appends failure result messages.
   *
   * @returns Number of tasks marked as failed
   */
  async cleanupRunningTasksOnShutdown(): Promise<number> {
    try {
      logger.info('Marking running tasks as failed...')

      // Get all tasks directly from repository (bypasses access control)
      const allTasks = await this.taskRepository.listAll()

      let markedCount = 0
      for (const task of allTasks) {
        // Check if task is running
        if (task.status === 'running') {
          // Pass task directly to bypass visibility-scoped fetch
          const marked = await this.markTaskAsFailed(task.taskId, 'Server shutdown', task as Task)
          if (marked) {
            markedCount++
          }
        }
      }

      if (markedCount > 0) {
        logger.info(`Marked ${markedCount} running tasks as failed`)
      } else {
        logger.info('No running tasks to mark as failed')
      }

      return markedCount
    } catch (error) {
      logger.error({ err: error }, 'Error marking tasks as failed during shutdown')
      return 0
    }
  }

  /**
   * Mark a running task as failed with a specific reason.
   * Appends a failure result message and updates task status to 'error'.
   * Used for system-level failures like server shutdown/restart.
   *
   * @param taskId - Task ID
   * @param reason - Reason for failure (e.g., 'Server shutdown', 'Recovered from server restart')
   * @param existingTask - Optional task object (bypasses visibility-scoped fetch for system operations)
   * @returns True if the task was running and was marked as failed
   */
  private async markTaskAsFailed(
    taskId: string,
    reason: string,
    existingTask?: Task,
  ): Promise<boolean> {
    const taskLogger = logger.child({ taskId })

    try {
      // Use provided task or fetch it (fetch goes through visibility scoping)
      const task = existingTask ?? (await this.taskRepository.get(taskId))
      if (!task) {
        taskLogger.debug('Task not found when marking task as failed')
        return false
      }

      // Only mark running tasks as failed
      if (task.status !== 'running') {
        taskLogger.debug({ status: task.status }, 'Task not in running state, skipping')
        return false
      }

      const turnNumber = 0

      // Agent session ID is ephemeral — not persisted
      const agentSessionId = undefined

      // Build failure result message
      const resultMessage: ResultAgentMessage = {
        uuid: randomUUID(),
        type: 'result',
        userType: 'system',
        turnNumber,
        timestamp: new Date().toISOString(),
        turnStatus: 'failed',
        turnResult: reason,
        agentSessionId,
        willNotResume: true,
        runMetrics: {
          costUsd: 0,
          inputTokens: 0,
          outputTokens: 0,
          contextTokens: 0,
        },
      }

      // Append result message
      await this.messageRepository.append(taskId, resultMessage)

      // Mark task as failed
      task.status = 'error'
      task.lastMessageAt = new Date()

      taskLogger.info(
        {
          turnNumber,
          reason,
        },
        'Marked task as failed',
      )

      // Save the task
      await this.taskRepository.save(task)

      // Emit taskTurn error event
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'taskTurn',
        subType: 'error',
        agentType: task.agentType,
        turnNumber,
        payload: {
          status: 'error',
          previousStatus: 'running',
          turnNumber,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        isError: true,
      } as TaskTurnEvent)

      return true
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to mark task as failed')
      return false
    }
  }
}
