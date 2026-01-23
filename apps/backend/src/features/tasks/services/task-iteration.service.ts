import { randomUUID } from 'node:crypto'
import type {
  AgentEvent,
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
import {
  agentCostUsdTotal,
  agentInputTokensTotal,
  agentOutputTokensTotal,
  taskIterationDurationSeconds,
  taskIterationTotal,
} from '../../metrics'
import { WorkspaceCoordinator } from '../../workspace/services/workspace-coordinator.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
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
 * - Agent event processing (handled directly in AgentController)
 * - Message persistence (belongs in MessageService)
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskIterationService {
  private unsubscribe?: (() => void) | undefined

  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(WorkspaceCoordinator) private readonly workspaceCoordinator: WorkspaceCoordinator,
    @inject(EventService) private readonly eventService: EventService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
  ) {}

  /**
   * Start listening for agent.completed events from EventService.
   * Called once during app initialization.
   * This enables automatic iteration completion when sandbox emits agent.completed via Redis.
   */
  startEventSubscription(): void {
    if (this.unsubscribe) {
      logger.warn('TaskIterationService: Event subscription already started')
      return
    }

    this.unsubscribe = this.eventService.subscribeAll(async (event) => {
      if (event.type === 'agent') {
        try {
          switch (event.subType) {
            case 'started':
              await this.handleAgentStartedEvent(event as AgentEvent)
              break
            case 'message':
              await this.handleAgentMessageEvent(event as AgentEvent)
              break
            case 'completed':
              await this.handleAgentCompletedEvent(event as AgentEvent)
              break
            case 'error':
              await this.handleAgentErrorEvent(event as AgentEvent)
              break
          }
        } catch (error) {
          logger.warn(
            {
              taskId: event.taskId,
              eventId: event.eventId,
              subType: event.subType,
              error: error instanceof Error ? error.message : String(error),
            },
            'Failed to handle agent event',
          )
        }
      }
    })
    logger.info('TaskIterationService: Started event subscription for agent events')
  }

  /**
   * Stop event subscription (for graceful shutdown)
   */
  stopEventSubscription(): void {
    if (this.unsubscribe) {
      this.unsubscribe()
      this.unsubscribe = undefined
      logger.info('TaskIterationService: Stopped event subscription')
    }
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
   * This replaces the explicit API call from sandbox.
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

    // Perform post-processing (git metadata + screenshots)
    await this.performPostProcessing(task as Task, activeIteration as TaskIteration)

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

    // Cleanup workspace for background iterations (fire-and-forget)
    if (activeIteration.background) {
      this.cleanupWorkspaceForBackgroundIteration(taskId, taskLogger)
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

    // Perform post-processing (capture git state even on error)
    await this.performPostProcessing(task as Task, activeIteration as TaskIteration)

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

    // Cleanup workspace for background iterations (fire-and-forget)
    if (activeIteration.background) {
      this.cleanupWorkspaceForBackgroundIteration(taskId, taskLogger)
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
   * Cleanup workspace for a background iteration.
   * Called after the iteration completes or errors. Runs in background (fire-and-forget).
   * Task remains in its current status (open/error) — NOT closed after cleanup.
   */
  private cleanupWorkspaceForBackgroundIteration(taskId: string, taskLogger: Logger): void {
    taskLogger.info({ taskId }, 'Background iteration complete, scheduling workspace cleanup')
    // Fire-and-forget cleanup
    this.workspaceCoordinator.deleteWorkspace(taskId).catch((error) => {
      taskLogger.error(
        { error: error instanceof Error ? error.message : String(error) },
        'Failed to cleanup workspace for background iteration',
      )
    })
  }

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
    background?: boolean,
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
      background,
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
   * Capture git metadata for an iteration (status, changes, commit hash).
   *
   * @param taskId - Task ID
   * @param iteration - Iteration to populate with git data
   */
  async captureGitMetadata(taskId: string, iteration: TaskIteration): Promise<void> {
    const taskLogger = logger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      const handle = await this.runtimeManager.ensureWorkspace(taskId)
      const { driver } = handle

      // Get git status for sync state
      iteration.gitStatus = await driver.getGitStatus()
      taskLogger.debug(
        {
          syncStatus: iteration.gitStatus?.status,
          commitsAhead: iteration.gitStatus?.commitsAhead,
          commitsBehind: iteration.gitStatus?.commitsBehind,
          hasUncommittedChanges: iteration.gitStatus?.hasUncommittedChanges,
        },
        'Captured git status for iteration',
      )

      // Get git changes for working directory state
      iteration.gitChanges = await driver.getGitChanges()
      taskLogger.debug(
        {
          currentBranch: iteration.gitChanges?.currentBranch,
          status: iteration.gitChanges?.status,
          additions: iteration.gitChanges?.additions,
          deletions: iteration.gitChanges?.deletions,
          modifications: iteration.gitChanges?.modifications,
        },
        'Captured git changes for iteration',
      )

      // Get current commit hash
      iteration.commitHash = (await driver.getCurrentCommitHash()) || undefined
      taskLogger.debug(
        {
          commitHash: iteration.commitHash,
        },
        'Captured commit hash for iteration',
      )
    } catch (error) {
      taskLogger.warn(
        {
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to capture git metadata for iteration',
      )
    }
  }

  /**
   * Capture commit hash for iteration if not already set.
   *
   * @param taskId - Task ID
   * @param iteration - Iteration to update
   */
  async captureCommitHashIfNeeded(taskId: string, iteration: TaskIteration): Promise<void> {
    if (iteration.checkpointCommitHash) {
      return
    }

    const taskLogger = logger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      const handle = await this.runtimeManager.ensureWorkspace(taskId)
      const commitHash = await handle.driver.getCurrentCommitHash()
      if (commitHash) {
        iteration.checkpointCommitHash = commitHash
      }
    } catch (error) {
      taskLogger.debug(
        {
          error: error instanceof Error ? error.message : String(error),
        },
        'Could not capture commit hash',
      )
    }
  }

  /**
   * Perform all post-processing steps for an iteration:
   * 1. Capture git metadata
   * 2. Trigger screenshot capture (async, non-blocking)
   *
   * @param task - Task containing the iteration
   * @param iteration - Iteration to post-process
   */
  async performPostProcessing(task: Task, iteration: TaskIteration): Promise<void> {
    await this.captureGitMetadata(task.taskId, iteration)

    await this.captureCommitHashIfNeeded(task.taskId, iteration)
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

  /**
   * Recover orphaned active iterations from previous server runs.
   * Called during application startup to clean up iterations that were
   * left in 'active' or 'created' state due to server crash or unexpected shutdown.
   *
   * Smart recovery behavior:
   * - If workspace is 'initializing' or 'ready': Leave iteration for auto-resume
   *   (TaskExecutionCoordinator will handle via environment:ready event)
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
