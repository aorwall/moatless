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
import {
  createGitCheckpoint,
  getCurrentBranch,
  getCurrentCommitHash,
  getOriginCommitCounts,
  hasUncommittedChanges,
  pushGitCheckpoint,
} from '../../../utils/git-utils'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/index'
import { PlaywrightScreenshotService } from '../../preview/services/playwright-screenshot.service'
import { RepositoryService } from '../../repositories/index'
import { FileGateway } from '../../workspace/services/file-gateway.service'
import { TaskRepository } from '../db/task.repository'

/**
 * TaskIterationService - Responsible for iteration lifecycle management
 *
 * Handles:
 * - Creating new task iterations with validation
 * - Applying results and errors to iterations
 * - Capturing git metadata (branch, commit hash, changes)
 * - Creating git checkpoints
 * - Pushing changes to remote
 * - Coordinating screenshot captures
 * - Emitting iteration events
 *
 * Does NOT handle:
 * - Agent execution (belongs in TaskExecutionCoordinator)
 * - Event subscription (belongs in TaskEventProjector)
 * - Message persistence (belongs in MessageService)
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskIterationService {
  constructor(
    @inject(FileGateway) private readonly fileGateway: FileGateway,
    @inject(PlaywrightScreenshotService)
    private readonly screenshotService: PlaywrightScreenshotService,
    @inject(EventService) private readonly eventService: EventService,
    @inject(RepositoryService) private readonly repositoryService: RepositoryService,
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
      // Get git status for sync state
      iteration.gitStatus = await this.fileGateway.getGitStatus(taskId)
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
      iteration.gitChanges = await this.fileGateway.getGitChanges(taskId)
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
      iteration.commitHash = (await this.fileGateway.getCurrentCommitHash(taskId)) || undefined
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
   * Create a git checkpoint for an iteration if there are uncommitted changes.
   *
   * @param task - Task containing the iteration
   * @param iteration - Iteration to checkpoint
   */
  async createCheckpoint(task: Task, iteration: TaskIteration): Promise<void> {
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      const workspacePath = await this.fileGateway.getWorkspacePath(task.taskId)

      // Check if there are changes to commit
      const hasChanges = await hasUncommittedChanges(workspacePath)
      if (!hasChanges) {
        taskLogger.info('No changes to checkpoint')
        return
      }

      // Create checkpoint commit
      const message = `Checkpoint: Iteration ${iteration.iterationNumber} - ${iteration.status} [${new Date().toISOString()}]`
      const { commitHash } = await createGitCheckpoint(workspacePath, message)

      if (commitHash) {
        iteration.checkpointCommitHash = commitHash
        taskLogger.info({ commitHash }, 'Created git checkpoint')
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to create git checkpoint')
      // Don't fail the iteration if checkpoint fails
    }
  }

  /**
   * Push any existing commits to remote after iteration.
   *
   * @param task - Task containing the iteration
   * @param iteration - Iteration that triggered the push
   */
  async pushChanges(task: Task, iteration: TaskIteration): Promise<void> {
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      // Check if repository has a remote URL (only remote repositories have this)
      const hasRemoteUrl = repository?.kind === 'remote' && repository.remote.url
      if (!hasRemoteUrl) {
        taskLogger.info('No remote URL configured, skipping push')
        return
      }

      const workspacePath = await this.fileGateway.getWorkspacePath(task.taskId)
      const currentBranch = await getCurrentBranch(workspacePath)

      // Check if there are commits to push
      const { ahead } = await getOriginCommitCounts(workspacePath)
      if (ahead === 0) {
        taskLogger.info('No commits to push')
        return
      }

      // Push to remote
      const { pushed, error } = await pushGitCheckpoint(workspacePath, currentBranch)

      if (pushed) {
        // Capture the HEAD commit that was pushed
        const headCommit = await getCurrentCommitHash(workspacePath)
        if (headCommit) {
          iteration.checkpointCommitHash = headCommit
        }
        iteration.checkpointPushed = true

        taskLogger.info(
          {
            branch: currentBranch,
            commitsAhead: ahead,
            headCommit,
          },
          'Pushed commits to remote',
        )
      } else if (error) {
        taskLogger.warn({ error }, 'Failed to push commits')
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to push changes')
      // Don't fail the iteration if push fails
    }
  }

  /**
   * Capture commit hash for iteration if not already set.
   * This is called as a fallback to ensure we have a commit hash even if
   * checkpoint creation was skipped.
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
      const commitHash = await this.fileGateway.getCurrentCommitHash(taskId)
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
   * 2. Create checkpoint (if enabled)
   * 3. Push changes (if enabled)
   * 4. Capture commit hash (fallback)
   * 5. Trigger screenshot capture (async, non-blocking)
   *
   * @param task - Task containing the iteration
   * @param iteration - Iteration to post-process
   */
  async performPostProcessing(task: Task, iteration: TaskIteration): Promise<void> {
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    await this.captureGitMetadata(task.taskId, iteration)

    if (task.gitCheckpointEnabled) {
      await this.createCheckpoint(task, iteration)
    }

    if (task.gitPushEnabled) {
      await this.pushChanges(task, iteration)
    }

    await this.captureCommitHashIfNeeded(task.taskId, iteration)

    // Screenshot capture runs in background (non-blocking)
    this.screenshotService
      .captureIterationScreenshotIfEnabled(task.taskId, iteration)
      .then((result) => {
        if (result?.screenshotFileId) {
          return this.updateTaskWithScreenshot(task, iteration, result.screenshotFileId)
        }
        return Promise.resolve()
      })
      .catch((error) => {
        taskLogger.warn(
          {
            error: error instanceof Error ? error.message : String(error),
            iterationNumber: iteration.iterationNumber,
          },
          'Failed to capture preview screenshot',
        )
      })
  }

  /**
   * Update task with screenshot file ID and emit screenshot update event.
   * This is called after successful screenshot capture to persist the file ID
   * and notify clients.
   *
   * @param task - Task containing the iteration
   * @param iteration - Iteration to update with screenshot
   * @param screenshotFileId - File ID of captured screenshot
   */
  private async updateTaskWithScreenshot(
    task: Task,
    iteration: TaskIteration,
    screenshotFileId: string,
  ): Promise<void> {
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      // Update the iteration with screenshot file ID
      iteration.screenshotFileId = screenshotFileId
      taskLogger.debug(
        {
          screenshotFileId,
        },
        'Screenshot file ID added to iteration',
      )

      // Get the task to update it
      const savedTask = await this.taskRepository.get(task.taskId)
      if (!savedTask) {
        taskLogger.warn('Task not found when saving screenshot')
        return
      }

      // Find and update the iteration with screenshot file ID
      const taskIteration = savedTask.iterations.find(
        (it) => it.iterationNumber === iteration.iterationNumber,
      )
      if (taskIteration) {
        taskIteration.screenshotFileId = screenshotFileId
      }

      // Save the task
      await this.taskRepository.save(savedTask)
      taskLogger.debug('Task saved with screenshot file ID')

      // Emit event for screenshot capture
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'taskIteration',
        subType: 'updated',
        agentType: task.agentType,
        iterationId: iteration.iterationNumber,
        payload: {
          status: taskIteration?.status || iteration.status,
          previousStatus: taskIteration?.status || iteration.status,
          iterationNumber: iteration.iterationNumber,
          screenshotFileId,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        isError: false,
      })
      taskLogger.debug(
        {
          screenshotFileId,
        },
        'Emitted screenshot update event',
      )
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to update task with screenshot')
      // Don't rethrow - this is non-blocking
    }
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
   * @returns True if an active iteration was found and marked as failed
   */
  async markIterationAsFailed(taskId: string, reason: string): Promise<boolean> {
    const taskLogger = logger.child({ taskId })

    try {
      // Get the task
      const task = await this.taskRepository.get(taskId)
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
   * left in 'active' state due to server crash or unexpected shutdown.
   *
   * @returns Number of iterations recovered
   */
  async recoverOrphanedIterations(): Promise<number> {
    try {
      logger.info('Checking for orphaned active iterations...')

      // Get all tasks directly from repository (bypasses access control)
      const allTasks = await this.taskRepository.listAll()

      let recoveredCount = 0
      for (const task of allTasks) {
        // Check if task has any active iteration
        const hasActiveIteration = task.iterations.some(
          (it: { status: string }) => it.status === 'active',
        )
        if (hasActiveIteration) {
          const marked = await this.markIterationAsFailed(
            task.taskId,
            'Recovered from server restart',
          )
          if (marked) {
            recoveredCount++
            logger.info(
              {
                taskId: task.taskId,
                taskName: task.name,
              },
              'Recovered orphaned iteration',
            )
          }
        }
      }

      if (recoveredCount > 0) {
        logger.info(`Recovered ${recoveredCount} orphaned active iterations`)
      } else {
        logger.info('No orphaned active iterations found')
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
          const marked = await this.markIterationAsFailed(task.taskId, 'Server shutdown')
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
      errorMessage = 'Error occurred during execution'
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
