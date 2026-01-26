import { randomUUID } from 'node:crypto'
import type {
  Event,
  MessageContext,
  SDKResultMessage,
  Task,
  TaskEvent,
  TaskIteration,
  TaskIterationEvent,
} from '@moatless/types'
import { ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Logger } from 'pino'
// Note: git operations are now delegated to the driver (see createCheckpoint and pushChanges methods)
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/index'
import { taskIterationDurationSeconds, taskIterationTotal } from '../../metrics'
import { TaskRepository } from '../db/task.repository'

/**
 * TaskIterationService - Responsible for iteration lifecycle management
 *
 * Handles:
 * - Creating new task iterations with validation
 * - Applying results and errors to iterations
 * - Capturing git metadata (branch, commit hash, changes)
 * - Coordinating screenshot captures
 * - Emitting iteration events
 *
 * Does NOT handle:
 * - Agent execution (belongs in TaskExecutionCoordinator)
 * - Agent event processing (handled in TaskEventHandler)
 * - Message persistence (belongs in MessageService)
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskIterationService {
  constructor(
    @inject(EventService) private readonly eventService: EventService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
  ) {}

  /**
   * Create a new task iteration with validation.
   *
   * Validates that either message, commandName, or contexts is provided.
   *
   * @param task - Task to create iteration for
   * @param message - Optional initial message
   * @param commandName - Optional command name
   * @param model - Optional model override
   * @param initialStatus - Initial iteration status (default: 'active')
   * @param agentMode - Agent mode (edit/plan)
   * @param contexts - Optional message contexts
   * @returns New TaskIteration
   * @throws ValidationError if no message, command, or context provided
   */
  createIteration(
    task: Task,
    message?: string,
    commandName?: string,
    model?: string,
    initialStatus: 'created' | 'active' = 'active',
    agentMode?: 'edit' | 'plan',
    contexts?: MessageContext[],
  ): TaskIteration {
    // Validate that either message, commandName, or contexts is provided
    if (!(message || commandName || (contexts && contexts.length > 0))) {
      throw new ValidationError(
        'Either initialMessage, commandName, or at least one context item must be provided for task iteration',
        { message: 'Missing required message, command, or context' },
      )
    }

    const now = new Date()
    const iterationNumber = task.iterations.length + 1

    const iteration: TaskIteration = {
      iterationNumber,
      startTime: now,
      status: initialStatus,
      messageCount: 1,
      costUsd: 0,
      initialMessage: message,
      commandName,
      model,
      agentMode,
    }

    logger.info(
      {
        taskId: task.taskId,
        iterationNumber: iteration.iterationNumber,
        status: iteration.status,
        model: iteration.model,
        agentMode: iteration.agentMode,
        commandName: iteration.commandName,
        hasMessage: Boolean(iteration.initialMessage),
      },
      'Task iteration created',
    )

    return iteration
  }

  /**
   * Apply a result payload to an iteration.
   * Determines status (completed/failed) based on result subtype and content.
   *
   * @param iteration - Iteration to update
   * @param event - Event containing result
   * @param resultPayload - SDK result message
   */
  applyResult(iteration: TaskIteration, event: Event, resultPayload: SDKResultMessage): void {
    const taskLogger = logger.child({
      taskId: event.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    // Update iteration cost
    iteration.costUsd = resultPayload.total_cost_usd ?? 0

    // Determine iteration status and result based on payload
    if (event.isError || resultPayload.is_error) {
      this.applyErrorResult(iteration, resultPayload)
    } else if (resultPayload.subtype === 'success') {
      this.applySuccessResult(iteration, resultPayload, taskLogger)
    } else {
      this.applyUnexpectedResult(iteration, resultPayload, taskLogger)
    }

    iteration.endTime = new Date()

    // Track iteration completion metrics
    const agentType = 'agentType' in event ? event.agentType : 'unknown'
    taskIterationTotal.labels({ status: iteration.status, agent_type: agentType }).inc()

    // Track iteration duration
    if (iteration.startTime) {
      const durationSeconds = (iteration.endTime.getTime() - iteration.startTime.getTime()) / 1000
      taskIterationDurationSeconds.labels({ agent_type: agentType }).observe(durationSeconds)
    }
  }

  /**
   * Apply an error payload to an iteration.
   *
   * @param iteration - Iteration to update
   * @param errorPayload - Error payload from event
   */
  applyError(
    iteration: TaskIteration,
    errorPayload: { error?: string; stackTrace?: string },
  ): void {
    iteration.status = 'failed'
    iteration.result = `${errorPayload.error || 'Agent execution failed'}${
      errorPayload.stackTrace ? `\nStack: ${errorPayload.stackTrace}` : ''
    }`
    iteration.endTime = new Date()
  }

  /**
   * Update iteration context tokens from message event.
   *
   * @param iteration - Iteration to update
   * @param contextTokens - Number of context tokens
   */
  updateContextTokens(iteration: TaskIteration, contextTokens: number): void {
    iteration.contextTokens = contextTokens
  }

  /**
   * Emit iteration and task status events.
   *
   * @param task - Task containing the iteration
   * @param iteration - Iteration that completed
   * @param previousTaskStatus - Previous task status (for detecting changes)
   */
  async emitEvents(
    task: Task,
    iteration: TaskIteration,
    previousTaskStatus: string,
  ): Promise<void> {
    // Emit iteration event if completed
    if (iteration.status === 'completed') {
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'taskIteration',
        subType: 'completed',
        agentType: task.agentType,
        iterationId: iteration.iterationNumber,
        payload: {
          status: 'completed',
          previousStatus: 'active',
          iterationNumber: iteration.iterationNumber,
          result: iteration.result,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        isError: false,
      } as TaskIterationEvent)
    }

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
  }

  /**
   * Emit iteration error event.
   *
   * @param task - Task containing the iteration
   * @param iteration - Iteration that failed
   */
  async emitErrorEvent(task: Task, iteration: TaskIteration): Promise<void> {
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'taskIteration',
      subType: 'error',
      agentType: task.agentType,
      iterationId: iteration.iterationNumber,
      payload: {
        status: 'error',
        previousStatus: 'active',
        iterationNumber: iteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: true,
    })
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
  async markIterationAsFailed(
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

  /**
   * Mark an active iteration as completed with a result message.
   * Used when user manually stops an agent.
   *
   * @param taskId - Task ID
   * @param result - Result message (e.g., 'Stopped by user')
   * @returns True if an active iteration was found and marked as completed
   */
  async markIterationAsCompleted(taskId: string, result: string): Promise<boolean> {
    const taskLogger = logger.child({ taskId })

    try {
      // Get the task
      const task = await this.taskRepository.get(taskId)
      if (!task) {
        taskLogger.debug('Task not found when marking iteration as completed')
        return false
      }

      // Find the active iteration
      const activeIteration = task.iterations.find((it) => it.status === 'active')
      if (!activeIteration) {
        taskLogger.debug('No active iteration found to mark as completed')
        return false
      }

      // Mark iteration as completed
      activeIteration.status = 'completed'
      activeIteration.result = result
      activeIteration.endTime = new Date()

      taskLogger.info(
        {
          iterationNumber: activeIteration.iterationNumber,
          result,
        },
        'Marked iteration as completed',
      )

      // Save the task
      await this.taskRepository.save(task)

      // Emit iteration completed event
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'taskIteration',
        subType: 'completed',
        agentType: task.agentType,
        iterationId: activeIteration.iterationNumber,
        payload: {
          status: 'completed',
          previousStatus: 'active',
          iterationNumber: activeIteration.iterationNumber,
          result,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        isError: false,
      } as TaskIterationEvent)

      return true
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to mark iteration as completed')
      return false
    }
  }

  // Private helper methods

  private applyErrorResult(iteration: TaskIteration, resultPayload: SDKResultMessage): void {
    iteration.status = 'failed'

    let errorMessage = 'Agent execution failed'
    if (resultPayload.subtype === 'error_max_turns') {
      errorMessage = 'Maximum number of turns exceeded'
      iteration.willNotResume = true
    } else if (resultPayload.subtype === 'error_during_execution') {
      // Extract actual errors from SDK payload
      const errors =
        'errors' in resultPayload && Array.isArray(resultPayload.errors)
          ? (resultPayload.errors as string[])
          : []
      errorMessage = errors.length > 0 ? errors.join('; ') : 'Error occurred during execution'
    }

    iteration.result =
      'result' in resultPayload && resultPayload.result !== null
        ? resultPayload.result
        : errorMessage
  }

  private applySuccessResult(
    iteration: TaskIteration,
    resultPayload: SDKResultMessage,
    taskLogger: Logger,
  ): void {
    // Type narrowing: only 'success' subtype has result field
    if (resultPayload.subtype !== 'success') {
      iteration.status = 'failed'
      iteration.result = 'Agent completed without producing output'
      return
    }

    const hasValidResult =
      resultPayload.result !== null &&
      resultPayload.result !== undefined &&
      resultPayload.result !== ''

    if (!hasValidResult) {
      iteration.status = 'failed'
      iteration.result = this.getEmptyResultMessage(resultPayload.result)
      taskLogger.warn(
        {
          resultValue: resultPayload.result,
          subtype: resultPayload.subtype,
        },
        'Agent completed without valid result',
      )
    } else {
      iteration.status = 'completed'
      iteration.result = resultPayload.result
    }
  }

  private getEmptyResultMessage(result: string | null | undefined): string {
    if (result === null || result === undefined) {
      return 'Agent completed successfully but returned no result'
    }
    if (result === '') {
      return 'Agent completed successfully with empty result'
    }
    return 'Agent completed without producing output'
  }

  private applyUnexpectedResult(
    iteration: TaskIteration,
    resultPayload: SDKResultMessage,
    taskLogger: Logger,
  ): void {
    iteration.status = 'failed'
    iteration.result = `Unexpected result subtype: ${resultPayload.subtype}`
    taskLogger.error('Unexpected non-error, non-success result subtype')
  }
}
