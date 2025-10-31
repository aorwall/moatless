import type { Event, SDKResultMessage, Task, TaskIteration } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/index'
import { TaskRepository } from '../db/task.repository'
import { TaskIterationService } from './task-iteration.service'

/**
 * TaskEventProjector - Reactive event handler for task updates
 *
 * Subscribes to the event stream and updates task state based on agent events.
 * This is the "reactive" side of task management - it listens for events and
 * projects them onto task state.
 *
 * Handles:
 * - Agent completion events (apply results to iterations)
 * - Agent error events (mark iterations as failed)
 * - Agent message events (update context tokens)
 * - Agent session ID extraction and storage
 *
 * This service should be initialized once during application bootstrap to start
 * listening for events.
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskEventProjector {
  constructor(
    @inject(EventService) private readonly eventService: EventService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(TaskIterationService) private readonly iterationService: TaskIterationService,
  ) {}

  /**
   * Subscribe to all events and handle relevant ones.
   * Should be called once during application initialization.
   */
  subscribe(): void {
    this.eventService.subscribeAll(async (event: Event) => {
      if (this.shouldHandleEvent(event)) {
        try {
          await this.handleEvent(event)
        } catch (error) {
          logger.error('Failed to handle event for task update', error as Error, {
            eventId: event.eventId,
            taskId: event.taskId,
            eventType: event.type,
          })
        }
      }
    })

    logger.info('TaskEventProjector subscribed to event stream')
  }

  /**
   * Determine if this event should trigger task updates.
   *
   * @param event - Event to check
   * @returns True if event should be handled
   */
  private shouldHandleEvent(event: Event): boolean {
    if (event.type === 'agent') {
      return (
        event.subType === 'completed' ||
        event.subType === 'error' ||
        (event.subType === 'message' && event.payload?.contextTokens !== undefined) ||
        event.payload?.agentSessionId !== undefined
      )
    }
    return false
  }

  /**
   * Handle an event by updating task state.
   *
   * @param event - Event to handle
   */
  private async handleEvent(event: Event): Promise<void> {
    const taskLogger = logger.child({
      taskId: event.taskId,
      eventType: event.type,
      eventId: event.eventId,
    })

    try {
      const task = await this.taskRepository.get(event.taskId)
      if (!task) {
        taskLogger.warn('Task not found for event, skipping update')
        return
      }

      const iteration = this.findIterationForEvent(task, event)
      if (!iteration) {
        taskLogger.warn('Iteration not found for event, skipping update')
        return
      }

      // Extract and store agentSessionId if present and not already set
      const agentSessionId =
        event.type === 'agent' && typeof event.payload?.agentSessionId === 'string'
          ? event.payload.agentSessionId
          : undefined

      if (agentSessionId && !iteration.agentSessionId) {
        iteration.agentSessionId = agentSessionId
        taskLogger.debug('Updated agent session ID from event', {
          sessionId: agentSessionId,
        })
      }

      // Process the event
      const wasHandled = await this.handleEventByType(task, iteration, event)

      // Save if event was handled OR if we extracted agent session ID
      if (wasHandled || agentSessionId) {
        await this.taskRepository.save(task)
      }

      taskLogger.debug('Successfully handled event for task update')
    } catch (error) {
      taskLogger.error('Failed to handle event', error as Error)
      throw error
    }
  }

  /**
   * Find the iteration that corresponds to this event.
   *
   * @param task - Task to search
   * @param event - Event to match
   * @returns Matching iteration or undefined
   */
  private findIterationForEvent(task: Task, event: Event): TaskIteration | undefined {
    const hasIterationId =
      (event.type === 'agent' || event.type === 'taskIteration') && 'iterationId' in event

    if (hasIterationId) {
      return task.iterations.find((it) => it.iterationNumber === event.iterationId)
    }

    // Default to last iteration
    return task.iterations[task.iterations.length - 1]
  }

  /**
   * Route event to specific handler based on type.
   *
   * @param task - Task being updated
   * @param iteration - Iteration being updated
   * @param event - Event to handle
   * @returns True if event was handled
   */
  private async handleEventByType(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<boolean> {
    if (event.type !== 'agent') {
      return false
    }

    if (event.subType === 'completed') {
      await this.handleResultEvent(task, iteration, event)
      return true
    }

    if (event.subType === 'error') {
      await this.handleErrorEvent(task, iteration, event)
      return true
    }

    if (event.subType === 'message') {
      await this.handleMessageEvent(task, iteration, event)
      return true
    }

    return false
  }

  /**
   * Handle agent completion events.
   * Updates iteration status and result, performs post-processing, emits events.
   *
   * @param task - Task to update
   * @param iteration - Iteration to update
   * @param event - Completion event
   */
  private async handleResultEvent(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<void> {
    const resultPayload = event.payload as SDKResultMessage
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    taskLogger.info('Handling RESULT event for task update', {
      costUsd: resultPayload.total_cost_usd,
      hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
      isError: event.isError,
      subtype: resultPayload.subtype,
    })

    // Apply result to iteration
    this.iterationService.applyResult(iteration, event, resultPayload)

    // Perform post-processing (git, screenshots)
    await this.iterationService.performPostProcessing(task, iteration)

    // Update task status based on iteration outcome
    const previousTaskStatus = task.status
    task.status = iteration.status === 'failed' ? 'error' : 'open'

    // Emit events for iteration completion and task status change
    await this.iterationService.emitEvents(task, iteration, previousTaskStatus)
  }

  /**
   * Handle agent error events.
   * Marks iteration as failed and captures git metadata.
   *
   * @param task - Task to update
   * @param iteration - Iteration to update
   * @param event - Error event
   */
  private async handleErrorEvent(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<void> {
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    taskLogger.error('Handling ERROR event for task update')

    // Apply error to iteration
    const errorPayload: { error?: string; stackTrace?: string } = {}
    if (event.payload?.error) {
      errorPayload.error = event.payload.error as string
    }
    if (event.payload?.stackTrace) {
      errorPayload.stackTrace = event.payload.stackTrace as string
    }
    this.iterationService.applyError(iteration, errorPayload)

    // Capture git data even for failed iterations
    await this.iterationService.captureGitMetadata(task.taskId, iteration)

    // Emit iteration error event
    await this.iterationService.emitErrorEvent(task, iteration)
  }

  /**
   * Handle agent message events that contain context token updates.
   *
   * @param task - Task to update
   * @param iteration - Iteration to update
   * @param event - Message event
   */
  private handleMessageEvent(task: Task, iteration: TaskIteration, event: Event): void {
    const contextTokens =
      event.type === 'agent' && typeof event.payload?.contextTokens === 'number'
        ? event.payload.contextTokens
        : undefined

    if (contextTokens !== undefined) {
      this.iterationService.updateContextTokens(iteration, contextTokens)

      logger.debug('Updated context tokens from MESSAGE event', {
        taskId: task.taskId,
        iterationNumber: iteration.iterationNumber,
        contextTokens,
      })
    }
  }
}
