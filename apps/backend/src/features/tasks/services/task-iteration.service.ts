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
import { ScreenshotOrchestrationService } from '../../../services/screenshot-orchestration.service'
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
import { RepositoryService } from '../../repositories/index'
import { FileGateway } from '../../workspace/index'

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
    @inject(ScreenshotOrchestrationService)
    private readonly screenshotService: ScreenshotOrchestrationService,
    @inject(EventService) private readonly eventService: EventService,
    @inject(RepositoryService) private readonly repositoryService: RepositoryService,
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

    logger.info('Task iteration created', {
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
      status: iteration.status,
      model: iteration.model,
      agentMode: iteration.agentMode,
      commandName: iteration.commandName,
      hasMessage: Boolean(iteration.initialMessage),
    })

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
      taskLogger.debug('Captured git status for iteration', {
        syncStatus: iteration.gitStatus?.status,
        commitsAhead: iteration.gitStatus?.commitsAhead,
        commitsBehind: iteration.gitStatus?.commitsBehind,
        hasUncommittedChanges: iteration.gitStatus?.hasUncommittedChanges,
      })

      // Get git changes for working directory state
      iteration.gitChanges = await this.fileGateway.getGitChanges(taskId)
      taskLogger.debug('Captured git changes for iteration', {
        currentBranch: iteration.gitChanges?.currentBranch,
        status: iteration.gitChanges?.status,
        additions: iteration.gitChanges?.additions,
        deletions: iteration.gitChanges?.deletions,
        modifications: iteration.gitChanges?.modifications,
      })

      // Get current commit hash
      iteration.commitHash = (await this.fileGateway.getCurrentCommitHash(taskId)) || undefined
      taskLogger.debug('Captured commit hash for iteration', {
        commitHash: iteration.commitHash,
      })
    } catch (error) {
      taskLogger.warn('Failed to capture git metadata for iteration', {
        error: error instanceof Error ? error.message : String(error),
      })
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
        taskLogger.info('Created git checkpoint', { commitHash })
      }
    } catch (error) {
      taskLogger.error('Failed to create git checkpoint', error as Error)
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
      if (!repository?.remoteUrl) {
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

        taskLogger.info('Pushed commits to remote', {
          branch: currentBranch,
          commitsAhead: ahead,
          headCommit,
        })
      } else if (error) {
        taskLogger.warn('Failed to push commits', { error })
      }
    } catch (error) {
      taskLogger.error('Failed to push changes', error as Error)
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
      taskLogger.debug('Could not capture commit hash', {
        error: error instanceof Error ? error.message : String(error),
      })
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
      .catch((error) => {
        taskLogger.warn('Failed to capture preview screenshot', {
          error: error instanceof Error ? error.message : String(error),
          iterationNumber: iteration.iterationNumber,
        })
      })
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
    taskLogger: ReturnType<typeof logger.child>,
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
      taskLogger.warn('Agent completed without valid result', {
        resultValue: resultPayload.result,
        subtype: resultPayload.subtype,
      })
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
    taskLogger: ReturnType<typeof logger.child>,
  ): void {
    iteration.status = 'failed'
    iteration.result = `Unexpected result subtype: ${resultPayload.subtype}`
    taskLogger.error('Unexpected non-error, non-success result subtype', {
      subtype: resultPayload.subtype,
      isError: resultPayload.is_error,
    })
  }
}
