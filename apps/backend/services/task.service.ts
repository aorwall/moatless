import { randomUUID } from 'node:crypto'
import path from 'node:path'
import type {
  Command,
  CreateTaskRequest,
  CreateVibeTaskRequest,
  Event,
  SDKResultMessage,
  Task,
  TaskIteration,
} from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'

import { filterAvailableCommands } from '../utils/command-conditions.js'
import {
  createGitCheckpoint,
  getCurrentBranch,
  getCurrentCommitHash,
  getOriginCommitCounts,
  hasUncommittedChanges,
  pushGitCheckpoint,
} from '../utils/git-utils.js'
import { logger } from '../utils/logger.js'
import type { AgentService } from './agent.service.js'
import type { CommandService } from './command.service.js'
import type { EventService } from './event.service.js'
import type { RepositoryService } from './repository.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

interface FileMetadata {
  originalFilename: string
  path: string
  mimeType: string
}

export class TaskService {
  private static readonly WORKSPACE_READY_TIMEOUT = 30000
  private static readonly WORKSPACE_CHECK_INTERVAL = 500
  private static readonly FILE_MIME_TYPES: Record<string, string> = {
    '.jpg': 'image/jpeg',
    '.jpeg': 'image/jpeg',
    '.png': 'image/png',
    '.gif': 'image/gif',
    '.webp': 'image/webp',
    '.pdf': 'application/pdf',
    '.txt': 'text/plain',
    '.md': 'text/markdown',
    '.json': 'application/json',
  }

  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
    private workspaceService: WorkspaceService,
    private commandService: CommandService,
    private repositoryService: RepositoryService,
    private agentService: AgentService,
  ) {
    // Subscribe to agent events for reactive task updates
    this.setupEventSubscription()
  }

  /**
   * Derive task status based on the last iteration's status
   * If the last iteration is "active", the task is considered "running"
   */
  private deriveTaskStatus(task: Task): Task {
    // Closed tasks should remain closed regardless of iteration status
    if (task.status === 'closed') {
      return task
    }

    if (!task.iterations || task.iterations.length === 0) {
      return task
    }

    // Get the last iteration
    const lastIteration = task.iterations[task.iterations.length - 1]

    // If the last iteration is active, the task is running
    if (lastIteration.status === 'active') {
      return {
        ...task,
        status: 'running',
      }
    }

    // For other statuses, keep the existing task status
    return task
  }

  /**
   * Setup event subscription for reactive task updates
   */
  private setupEventSubscription(): void {
    this.eventService.subscribeAll((event: Event) => {
      // Only handle events that should trigger task updates
      if (this.shouldHandleEvent(event)) {
        this.handleAgentEvent(event).catch((error) => {
          logger.error('Failed to handle agent event for task update', error as Error, {
            eventId: event.eventId,
            taskId: event.taskId,
            eventType: event.type,
          })
        })
      }
    })
  }

  /**
   * Determine if this event should trigger task updates
   */
  private shouldHandleEvent(event: Event): boolean {
    // Handle events that affect task state
    return (
      (event.type === 'agent' && event.subType === 'completed') ||
      (event.type === 'agent' && event.subType === 'error') ||
      (event.type === 'agent' &&
        event.subType === 'message' &&
        event.contextTokens !== undefined) ||
      event.agentSessionId !== undefined // Handle any event with agentSessionId
    )
  }

  /**
   * Handle agent events reactively to update tasks
   */
  private async handleAgentEvent(event: Event): Promise<void> {
    const taskLogger = logger.child({
      taskId: event.taskId,
      eventType: event.type,
      eventId: event.eventId,
    })

    try {
      const task = await this.storage.getTask(event.taskId)
      if (!task) {
        taskLogger.warn('Task not found for event, skipping update')
        return
      }

      // Find the iteration this event belongs to
      const iteration = event.iterationId
        ? task.iterations.find((it) => it.iterationNumber === event.iterationId)
        : task.iterations[task.iterations.length - 1] // Latest iteration if no iterationId

      if (!iteration) {
        taskLogger.warn('Iteration not found for event, skipping update', {
          iterationId: event.iterationId,
          totalIterations: task.iterations.length,
        })
        return
      }

      // Handle agentSessionId if present and not already set
      if (event.agentSessionId && !iteration.agentSessionId) {
        iteration.agentSessionId = event.agentSessionId
        taskLogger.debug('Updated agent session ID from event', { sessionId: event.agentSessionId })
      }

      // Handle different event types
      switch (event.type) {
        case 'agent':
          if (event.subType === 'completed') {
            await this.handleResultEvent(task, iteration, event)
          } else if (event.subType === 'error') {
            await this.handleErrorEvent(task, iteration, event)
          } else if (event.subType === 'message') {
            await this.handleMessageEvent(task, iteration, event)
          } else if (event.agentSessionId) {
            // For any other agent event type, just save if we updated agentSessionId
            await this.storage.saveTask(task)
          }
          break
        default:
          // For any other event type, just save if we updated agentSessionId
          if (event.agentSessionId) {
            await this.storage.saveTask(task)
          }
          break
      }

      taskLogger.debug('Successfully handled agent event for task update')
    } catch (error) {
      taskLogger.error('Failed to handle agent event', error as Error)
      throw error
    }
  }

  /**
   * Handle RESULT events from agent execution
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

    // Update iteration cost
    iteration.costUsd = resultPayload.total_cost_usd ?? 0

    // Check if the result event indicates an error
    if (event.isError || resultPayload.is_error) {
      // Handle error result
      iteration.status = 'failed'

      // Determine error message based on subtype
      let errorMessage = 'Agent execution failed'
      if (resultPayload.subtype === 'error_max_turns') {
        errorMessage = 'Maximum number of turns exceeded'
        // Mark as will not resume when max turns is reached
        iteration.willNotResume = true
      } else if (resultPayload.subtype === 'error_during_execution') {
        errorMessage = 'Error occurred during execution'
      }

      if ('result' in resultPayload && resultPayload.result !== null) {
        iteration.result = resultPayload.result
      } else {
        iteration.result = errorMessage
      }
    } else {
      // Check if we have a meaningful result
      // The result field only exists when subtype is 'success'
      if (resultPayload.subtype === 'success') {
        // TypeScript now knows this has a result field due to discriminated union
        const hasValidResult =
          resultPayload.result !== null &&
          resultPayload.result !== undefined &&
          resultPayload.result !== ''

        if (!hasValidResult) {
          // Treat missing or empty result as a failure
          iteration.status = 'failed'

          let errorMessage = 'Agent completed without producing output'
          if (resultPayload.result === null || resultPayload.result === undefined) {
            errorMessage = 'Agent completed successfully but returned no result'
          } else if (resultPayload.result === '') {
            errorMessage = 'Agent completed successfully with empty result'
          }

          iteration.result = errorMessage

          taskLogger.warn('Agent completed without valid result', {
            resultValue: resultPayload.result,
            subtype: resultPayload.subtype,
          })
        } else {
          // Handle successful result with valid content
          iteration.status = 'completed'
          iteration.result = resultPayload.result
        }
      } else {
        // This shouldn't happen - non-error, non-success result
        // But handle it gracefully
        iteration.status = 'failed'
        iteration.result = `Unexpected result subtype: ${resultPayload.subtype}`

        taskLogger.error('Unexpected non-error, non-success result subtype', {
          subtype: resultPayload.subtype,
          isError: resultPayload.is_error,
        })
      }
    }

    iteration.endTime = new Date()

    // Capture git data when iteration completes
    await this.captureGitDataForIteration(task.taskId, iteration)

    // Create git checkpoint if enabled (independent)
    if (task.gitCheckpointEnabled) {
      await this.createIterationCheckpoint(task, iteration)
    }

    // Push to remote if enabled (independent)
    if (task.gitPushEnabled) {
      await this.pushIterationChanges(task, iteration)
    }

    // Always capture the current commit hash for the iteration
    // (whether from checkpoint, manual commits, or existing state)
    if (!iteration.checkpointCommitHash) {
      try {
        const workspace = await this.workspaceService.getWorkspace(task.taskId)
        if (workspace) {
          const workspacePath = workspace.getWorkspacePath()
          const commitHash = await getCurrentCommitHash(workspacePath)
          if (commitHash) {
            iteration.checkpointCommitHash = commitHash
          }
        }
      } catch (error) {
        // Non-critical, just log
        taskLogger.debug('Could not capture commit hash', {
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    // Capture preview screenshot if server is configured and takeScreenshots is enabled (non-blocking)
    this.capturePreviewScreenshotIfEnabled(task.taskId, iteration).catch((error) => {
      taskLogger.warn('Failed to capture preview screenshot', {
        error: error instanceof Error ? error.message : String(error),
        iterationNumber: iteration.iterationNumber,
      })
    })

    // Update task status based on iteration result
    const previousStatus = task.status
    // Set error status if iteration failed (including missing results)
    const newStatus = iteration.status === 'failed' ? 'error' : 'open'
    task.status = newStatus

    // Save task with updated iteration and status
    await this.storage.saveTask(task)

    // Emit taskIteration event for completed iteration
    if (iteration.status === 'completed') {
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'taskIteration',
        subType: 'completed',
        payload: {
          status: 'completed',
          previousStatus: 'active',
          iterationNumber: iteration.iterationNumber,
          result: iteration.result,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        agentType: task.agentType,
        isError: false,
        iterationId: iteration.iterationNumber,
      })
    }

    // Emit task status change event if status actually changed
    if (previousStatus !== newStatus) {
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'task',
        subType: newStatus as 'open' | 'closed',
        payload: {
          status: newStatus,
          previousStatus,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        agentType: task.agentType,
        isError: newStatus === 'error',
      })
    }
  }

  /**
   * Handle ERROR events from agent execution
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

    // Save error to iteration
    iteration.status = 'failed'
    iteration.result = `${(event.payload?.error as string) || 'Agent execution failed'}${event.payload?.stackTrace ? '\nStack: ' + event.payload.stackTrace : ''}`
    iteration.endTime = new Date()

    // Capture git data even for failed iterations
    await this.captureGitDataForIteration(task.taskId, iteration)

    // Task status remains unchanged - only iteration status changes

    // Save task with updated iteration
    await this.storage.saveTask(task)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'taskIteration',
      subType: 'error',
      payload: {
        status: 'error',
        previousStatus: 'active',
        iterationNumber: iteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: task.agentType,
      isError: true,
    })
  }

  /**
   * Handle MESSAGE events that contain context token updates
   */
  private async handleMessageEvent(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<void> {
    // Update contextTokens if provided in the event
    if (event.contextTokens !== undefined) {
      iteration.contextTokens = event.contextTokens

      // Save task with updated context tokens
      await this.storage.saveTask(task)

      logger.debug('Updated context tokens from MESSAGE event', {
        taskId: task.taskId,
        iterationNumber: iteration.iterationNumber,
        contextTokens: event.contextTokens,
      })
    }
  }

  /**
   * Capture git status and changes for an iteration
   */
  private async captureGitDataForIteration(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<void> {
    const taskLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (!workspace) {
        taskLogger.warn('Workspace not found, skipping git data capture')
        return
      }

      // Get git status for sync state
      iteration.gitStatus = await workspace.getGitStatus()
      taskLogger.debug('Captured git status for iteration', {
        syncStatus: iteration.gitStatus?.status,
        commitsAhead: iteration.gitStatus?.commitsAhead,
        commitsBehind: iteration.gitStatus?.commitsBehind,
        hasUncommittedChanges: iteration.gitStatus?.hasUncommittedChanges,
      })

      // Get git changes for working directory state
      iteration.gitChanges = await workspace.getGitChanges()
      taskLogger.debug('Captured git changes for iteration', {
        currentBranch: iteration.gitChanges?.currentBranch,
        status: iteration.gitChanges?.status,
        additions: iteration.gitChanges?.additions,
        deletions: iteration.gitChanges?.deletions,
        modifications: iteration.gitChanges?.modifications,
      })

      // Get current commit hash
      iteration.commitHash = (await workspace.getCurrentCommitHash()) || undefined
      taskLogger.debug('Captured commit hash for iteration', {
        commitHash: iteration.commitHash,
      })
    } catch (error) {
      taskLogger.warn('Failed to capture git status/changes for iteration', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  /**
   * Capture preview screenshot asynchronously (non-blocking)
   */
  private async capturePreviewScreenshotAsync(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<void> {
    const taskLogger = logger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      const screenshotResult = await this.workspaceService.capturePreviewScreenshot(
        taskId,
        iteration.iterationNumber,
      )

      if (screenshotResult) {
        iteration.screenshotPath = screenshotResult.screenshotPath
        iteration.thumbnailPath = screenshotResult.thumbnailPath
        taskLogger.info('Screenshot and thumbnail paths added to iteration', {
          screenshotPath: screenshotResult.screenshotPath,
          thumbnailPath: screenshotResult.thumbnailPath,
        })

        // Save the updated iteration with screenshot and thumbnail paths
        const task = await this.storage.getTask(taskId)
        if (task) {
          // Find and update the iteration with screenshot paths
          const taskIteration = task.iterations.find(
            (it) => it.iterationNumber === iteration.iterationNumber,
          )
          if (taskIteration) {
            taskIteration.screenshotPath = screenshotResult.screenshotPath
            taskIteration.thumbnailPath = screenshotResult.thumbnailPath
          }
          await this.storage.saveTask(task)
          taskLogger.debug('Task saved with screenshot and thumbnail paths')

          // Emit event for screenshot capture
          await this.eventService.emitEvent({
            taskId: task.taskId,
            type: 'taskIteration',
            subType: 'updated',
            payload: {
              status: taskIteration?.status || iteration.status,
              previousStatus: taskIteration?.status || iteration.status,
              iterationNumber: iteration.iterationNumber,
              screenshotPath: screenshotResult.screenshotPath,
              thumbnailPath: screenshotResult.thumbnailPath,
            },
            eventId: randomUUID(),
            timestamp: new Date(),
            agentType: task.agentType,
            isError: false,
          })
          taskLogger.info('Emitted screenshot update event', {
            screenshotPath: screenshotResult.screenshotPath,
            thumbnailPath: screenshotResult.thumbnailPath,
          })
        }
      } else {
        taskLogger.debug('No screenshot captured (server not configured or failed)')
      }
    } catch (error) {
      taskLogger.error('Failed to capture preview screenshot', error as Error)
      // Don't rethrow - this is non-blocking
    }
  }

  /**
   * Capture preview screenshot if enabled in repository configuration (non-blocking)
   */
  private async capturePreviewScreenshotIfEnabled(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<void> {
    const taskLogger = logger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      // Get the task to access repository ID
      const task = await this.storage.getTask(taskId)
      if (!task) {
        taskLogger.warn('Task not found when checking screenshot setting')
        return
      }

      // Get the repository configuration
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        taskLogger.warn('Repository not found when checking screenshot setting')
        return
      }

      // Check if screenshots are enabled in the repository configuration
      if (!repository.previewConfig?.takeScreenshots) {
        taskLogger.debug('Screenshots disabled for repository, skipping capture')
        return
      }

      // Screenshots are enabled, proceed with capture
      await this.capturePreviewScreenshotAsync(taskId, iteration)
    } catch (error) {
      taskLogger.error('Failed to check screenshot setting or capture screenshot', error as Error)
      // Don't rethrow - this is non-blocking
    }
  }

  /**
   * Create a git checkpoint for an iteration
   */
  private async createIterationCheckpoint(task: Task, iteration: TaskIteration): Promise<void> {
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      const workspace = await this.workspaceService.getWorkspace(task.taskId)
      if (!workspace) {
        taskLogger.warn('Workspace not found, skipping checkpoint')
        return
      }

      const workspacePath = workspace.getWorkspacePath()

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
   * Push any existing commits to remote after iteration
   */
  private async pushIterationChanges(task: Task, iteration: TaskIteration): Promise<void> {
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

      const workspace = await this.workspaceService.getWorkspace(task.taskId)
      if (!workspace) {
        taskLogger.warn('Workspace not found, skipping push')
        return
      }

      const workspacePath = workspace.getWorkspacePath()
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

  async createTask(params: CreateTaskRequest): Promise<Task> {
    logger.info('Starting task creation', {
      agentType: params.agentType,
      repositoryId: params.repositoryId,
      hasBranchName: !!params.branchName,
      hasMessage: !!params.message,
      hasFilePaths: !!(params.filePaths && params.filePaths.length > 0),
      start: params.start ?? true,
      agentMode: params.agentMode,
      name: params.name,
      description: params.description,
    })

    // Validate name format if provided
    if (params.name && !/^[a-zA-Z0-9-]+$/.test(params.name)) {
      throw new ValidationError(
        'Task name must be URL-friendly (alphanumeric characters and dashes only)',
        { name: 'Invalid format' },
      )
    }

    const now = new Date()
    const task: Task = {
      taskId: randomUUID(),
      createdAt: now,
      lastMessageAt: now,
      agentType: params.agentType,
      repositoryId: params.repositoryId,
      status: 'open',
      numTurns: 0,
      totalCostUsd: 0.0,
      iterations: [],
      templateId: params.templateId,
      vibe: params.templateId ? true : undefined,
      name: params.name,
      description: params.description,
      gitCheckpointEnabled: params.gitCheckpointEnabled,
      gitPushEnabled: params.gitPushEnabled,
    }

    // Create task iteration with 'created' status if message is provided
    if (params.message) {
      const iteration = await this.createTaskIteration(
        task,
        params.message,
        undefined,
        params.model,
        undefined,
        params.filePaths,
        'created',
        params.agentMode, // agentMode from create task request
      )
      task.iterations.push(iteration)
    }

    try {
      await this.storage.saveTask(task)

      // Emit task created event
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'task',
        subType: 'open',
        payload: {
          status: 'open',
          previousStatus: 'created',
        },
        eventId: randomUUID(),
        timestamp: now,
        agentType: params.agentType,
        isError: false,
      })

      logger.info('Task metadata created successfully', {
        taskId: task.taskId,
        agentType: params.agentType,
        repositoryId: params.repositoryId,
        hasInitialIteration: !!params.message,
      })
    } catch (error) {
      logger.error('Failed to save task metadata', error as Error)
      throw error
    }

    try {
      const workspace = await this.workspaceService.createWorkspace(
        task.taskId,
        params.repositoryId,
        params.originalBranch || params.branchName, // Use originalBranch if provided, fallback to branchName for backward compatibility
        params.newBranch, // When set, create branch with this name and also worktree if we use worktree with a filepath normalized name
      )
      task.workspacePath = workspace.getWorkspacePath()

      // Save task with workspace path
      await this.storage.saveTask(task)
    } catch (error) {
      logger.error('Failed to create workspace', error as Error, {
        taskId: task.taskId,
        repositoryId: params.repositoryId,
      })
      throw error
    }

    // Return task immediately - workspace setup will run in background
    logger.info('Task creation completed successfully', { taskId: task.taskId })

    // Setup workspace and optionally start task in background
    this.setupWorkspaceInBackground(task.taskId, params.start, params.message)

    return task
  }

  /**
   * Sets up workspace and optionally starts task in background
   */
  private async setupWorkspaceInBackground(
    taskId: string,
    start?: boolean,
    message?: string,
  ): Promise<void> {
    try {
      logger.info('Starting background workspace setup', { taskId })

      // Setup workspace
      await this.workspaceService.setupWorkspace(taskId)

      // Check if workspace setup failed
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace && workspace.getStatus() === 'error') {
        // Workspace setup failed, record failure in iteration
        const task = await this.storage.getTask(taskId)
        if (task && task.iterations.length > 0) {
          const lastIteration = task.iterations[task.iterations.length - 1]
          if (lastIteration.status === 'created') {
            lastIteration.status = 'failed'
            const environment = workspace.getEnvironment()
            const errorMessage = environment.getError() || 'Environment setup failed'
            lastIteration.result = `Environment setup failed: ${errorMessage}`
            lastIteration.endTime = new Date()

            await this.storage.saveTask(task)

            // Emit error event for failed setup
            await this.eventService.emitEvent({
              eventId: randomUUID(),
              taskId,
              type: 'taskIteration',
              subType: 'error',
              isError: true,
              agentType: task.agentType,
              timestamp: new Date(),
              payload: {
                status: 'error',
                previousStatus: 'created',
                iterationNumber: lastIteration.iterationNumber,
              },
            })
          }
        }

        logger.error('Workspace setup failed, skipping task start', { taskId })
        return // Don't try to start the task
      }

      // Determine if we should start immediately
      const shouldStart = (start ?? true) && !!message

      // If we should start immediately, start the task
      if (shouldStart) {
        try {
          await this.startTask(taskId)
        } catch (error) {
          logger.error('Failed to start task after workspace setup', error as Error, { taskId })

          // Update task status to error when agent fails to start
          try {
            const task = await this.storage.getTask(taskId)
            if (task) {
              // Only update iteration status to failed, not task status
              let lastIteration: any = null
              if (task.iterations.length > 0) {
                lastIteration = task.iterations[task.iterations.length - 1]
                if (lastIteration.status === 'active') {
                  lastIteration.status = 'failed'
                  lastIteration.result = `Failed to start task after workspace setup: ${error instanceof Error ? error.message : String(error)}`
                  lastIteration.endTime = new Date()
                }
              }

              await this.storage.saveTask(task)

              // Emit error event only if there's a last iteration
              if (lastIteration) {
                await this.eventService.emitEvent({
                  eventId: randomUUID(),
                  taskId,
                  type: 'taskIteration',
                  subType: 'error',
                  isError: true,
                  agentType: task.agentType,
                  timestamp: new Date(),
                  payload: {
                    status: 'error',
                    previousStatus: 'active',
                    iterationNumber: lastIteration.iterationNumber,
                  },
                })
              }
            }
          } catch (updateError) {
            logger.error(
              'Failed to update task status after agent start failure',
              updateError as Error,
              { taskId },
            )
          }
          // Don't throw here - workspace was set up successfully
        }
      }

      logger.info('Background workspace setup completed', { taskId, started: shouldStart })
    } catch (error) {
      logger.error('Background workspace setup failed', error as Error, { taskId })

      // Update the iteration status to failed if it's not completed
      try {
        const task = await this.storage.getTask(taskId)
        if (task && task.iterations.length > 0) {
          const lastIteration = task.iterations[task.iterations.length - 1]

          // Only update if iteration is not completed (could be 'created' or 'active')
          if (lastIteration.status !== 'completed' && lastIteration.status !== 'failed') {
            const previousStatus = lastIteration.status
            lastIteration.status = 'failed'
            lastIteration.result = `Workspace setup failed: ${error instanceof Error ? error.message : String(error)}`
            lastIteration.endTime = new Date()

            await this.storage.saveTask(task)

            // Emit iteration error event
            await this.eventService.emitEvent({
              eventId: randomUUID(),
              taskId,
              type: 'taskIteration',
              subType: 'error',
              isError: true,
              agentType: task.agentType,
              timestamp: new Date(),
              payload: {
                status: 'error',
                previousStatus,
                iterationNumber: lastIteration.iterationNumber,
              },
            })
          }

          // Also emit task event for overall task status
          await this.eventService.emitEvent({
            eventId: randomUUID(),
            taskId,
            type: 'task',
            subType: 'open',
            isError: true,
            agentType: task.agentType,
            timestamp: new Date(),
            payload: {
              status: 'open',
              previousStatus: task.status,
            },
          })
        }
      } catch (updateError) {
        logger.error(
          'Failed to update task status after workspace setup failure',
          updateError as Error,
          { taskId },
        )
      }
    }
  }

  /**
   * Starts a task by activating the last iteration with 'created' status
   */
  async startTask(taskId: string): Promise<void> {
    logger.info('Starting task', { taskId })

    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Check if last iteration has status 'created'
    const lastIteration = task.iterations[task.iterations.length - 1]

    if (!lastIteration || lastIteration.status !== 'created') {
      throw new ValidationError(
        'Last iteration does not have status "created". Cannot start task.',
        { status: 'Last iteration not in created state' },
      )
    }

    // Wait for workspace to be ready
    const maxWaitTime = TaskService.WORKSPACE_READY_TIMEOUT
    const checkInterval = TaskService.WORKSPACE_CHECK_INTERVAL
    const startWait = Date.now()

    while (Date.now() - startWait < maxWaitTime) {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace) {
        const status = workspace.getStatus()
        if (status === 'ready') {
          break
        }
        if (status === 'error') {
          const environment = workspace.getEnvironment()
          const errorMessage = environment.getError() || 'Environment setup failed'
          throw new Error(`Cannot start task - workspace setup failed: ${errorMessage}`)
        }
      }
      await new Promise((resolve) => setTimeout(resolve, checkInterval))
    }

    // Update iteration status to active
    lastIteration.status = 'active'

    // Save task with updated statuses
    await this.storage.saveTask(task)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'taskIteration',
      subType: 'active',
      payload: {
        status: 'active',
        previousStatus: lastIteration.status,
        iterationNumber: lastIteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: task.agentType,
      isError: false,
    })

    // Execute the agent with the iteration's initial message
    if (lastIteration.initialMessage || lastIteration.commandName) {
      const agentRequest = {
        taskId,
        message: lastIteration.initialMessage || '',
        iteration: lastIteration,
        agentType: 'claude-code',
        resumeSessionId: undefined,
        model: lastIteration.model,
      }

      const executeTask = (async () => {
        for await (const event of this.agentService.executeAgent(agentRequest)) {
          // Emit agent events to event service
          await this.eventService.emitEvent(event)
        }
      })().catch(async (error) => {
        logger.error('Failed to execute agent during task start', error as Error, {
          taskId,
          iterationNumber: lastIteration.iterationNumber,
        })

        // Update status to error on agent failure
        try {
          const updatedTask = await this.storage.getTask(taskId)
          if (updatedTask) {
            const updatedIteration = updatedTask.iterations.find(
              (it) => it.iterationNumber === lastIteration.iterationNumber,
            )
            if (updatedIteration) {
              // Only update iteration status, not task status
              updatedIteration.status = 'failed'
              updatedIteration.result = `Agent failed to start: ${error instanceof Error ? error.message : String(error)}`
              updatedIteration.endTime = new Date()

              await this.storage.saveTask(updatedTask)

              // Emit error event
              await this.eventService.emitEvent({
                eventId: randomUUID(),
                taskId,
                type: 'taskIteration',
                subType: 'error',
                isError: true,
                agentType: updatedTask.agentType,
                timestamp: new Date(),
                payload: {
                  status: 'error',
                  previousStatus: lastIteration.status,
                  iterationNumber: lastIteration.iterationNumber,
                },
              })
            }
          }
        } catch (saveError) {
          logger.error('Failed to update task status after agent error', saveError as Error)
        }
      })

      // Track the background agent execution
      this.workspaceService.trackBackgroundTask(taskId, executeTask)
    }

    logger.info('Task started successfully', {
      taskId,
      iterationNumber: lastIteration.iterationNumber,
    })
  }

  async createVibeTask(params: CreateVibeTaskRequest): Promise<string> {
    logger.info('Starting vibe task creation from template', {
      agentType: params.agentType,
      templateId: params.templateId,
      hasMessage: !!params.message,
      model: params.model,
      hasFilePaths: !!(params.filePaths && params.filePaths.length > 0),
      start: params.start,
    })

    // Generate task ID first (we'll need it for the repository name if not provided)
    const taskId = randomUUID()

    // Auto-generate repository name using template ID + first 8 chars of task ID
    const repositoryName = `${params.templateId}-task-${taskId.slice(0, 8)}`

    logger.info('Generated repository name', {
      repositoryName,
      taskId: taskId.slice(0, 8),
    })

    // Create a new repository from the template - let RepositoryService handle all template logic
    const repository = await this.repositoryService.createRepository({
      name: repositoryName,
      description: `Created from template: ${params.templateId}`,
      sourceType: 'template',
      templateId: params.templateId,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        // setupCommands will be handled by RepositoryService from template definition
      },
      // previewConfig will be handled by RepositoryService from template definition
    })

    // Create a task with the new repository
    // Vibe tasks always start immediately
    const createTaskParams: CreateTaskRequest = {
      agentType: params.agentType,
      repositoryId: repository.id,
      message: params.message,
      start: params.start,
      model: params.model,
      filePaths: params.filePaths, // Support file uploads for context
      templateId: params.templateId,
      name: params.name,
      description: params.description,
    }

    const task = await this.createTask(createTaskParams)
    return task.taskId
  }

  async getTask(taskId: string): Promise<Task> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    return this.deriveTaskStatus(task)
  }

  async listTasks(
    limit = 50,
    offset = 0,
    status?: string,
  ): Promise<{ tasks: Task[]; total: number }> {
    const taskKeys = await this.storage.list('tasks/')
    const tasks: Task[] = []

    for (const key of taskKeys) {
      const taskId = key.replace('tasks/', '')
      const task = await this.storage.getTask(taskId)
      if (task) {
        // Apply status derivation
        const taskWithDerivedStatus = this.deriveTaskStatus(task)

        // Filter by status if provided, otherwise exclude closed tasks by default
        if (status) {
          // If status is explicitly provided, only include tasks with that status
          if (taskWithDerivedStatus.status === status) {
            tasks.push(taskWithDerivedStatus)
          }
        } else {
          // If no status filter, exclude closed tasks by default
          if (taskWithDerivedStatus.status !== 'closed') {
            tasks.push(taskWithDerivedStatus)
          }
        }
      }
    }

    // Sort by last message time (most recent first)
    tasks.sort((a, b) => b.lastMessageAt.getTime() - a.lastMessageAt.getTime())

    // Store total count before pagination
    const total = tasks.length

    // Apply pagination
    const paginatedTasks = tasks.slice(offset, offset + limit)

    return { tasks: paginatedTasks, total }
  }

  /**
   * Creates a new task iteration without starting it
   */
  private async createTaskIteration(
    task: Task,
    message?: string,
    commandName?: string,
    model?: string,
    fileIds?: string[],
    filePaths?: string[],
    initialStatus: 'created' | 'active' = 'active',
    agentMode?: 'edit' | 'plan',
  ): Promise<TaskIteration> {
    // Validate that either message or commandName is provided
    if (!message && !commandName) {
      throw new ValidationError(
        'Either initialMessage or commandName must be provided for task iteration',
        { message: 'Missing required message or command' },
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

    // Add file attachments if provided
    if (filePaths && filePaths.length > 0) {
      iteration.attachments = []
      for (const filePath of filePaths) {
        // Extract filename from path
        const filename = path.basename(filePath)
        // Try to determine mime type from extension
        const ext = path.extname(filename).toLowerCase()
        const mimeType = TaskService.FILE_MIME_TYPES[ext] || 'application/octet-stream'

        iteration.attachments.push({
          filename,
          path: filePath,
          mimeType,
        })
      }
    } else if (fileIds && fileIds.length > 0) {
      // Backward compatibility with fileIds
      iteration.attachments = []
      for (const fileId of fileIds) {
        // Get file metadata
        const fileMetadata = await this.storage.get<FileMetadata>(
          `uploads/${task.taskId}/${fileId}`,
        )
        if (fileMetadata) {
          iteration.attachments.push({
            fileId,
            filename: fileMetadata.originalFilename,
            path: fileMetadata.path,
            mimeType: fileMetadata.mimeType,
          })
        }
      }
    }

    logger.info('Task iteration created', {
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
      status: iteration.status,
      model: iteration.model,
      agentMode: iteration.agentMode,
      commandName: iteration.commandName,
      hasMessage: !!iteration.initialMessage,
      hasAttachments: !!(iteration.attachments && iteration.attachments.length > 0),
    })

    return iteration
  }

  async sendCommand(
    taskId: string,
    commandName: string,
    resume = true,
    model?: string,
    filePaths?: string[],
  ): Promise<void> {
    const command = await this.commandService.getCommand(commandName)
    if (!command) {
      throw new ValidationError(`Command '${commandName}' not found`, 'commandName')
    }

    // Use the command's prompt as the message
    await this.sendMessage(
      taskId,
      command.prompt,
      resume,
      model,
      undefined,
      filePaths,
      commandName,
      command.agentMode || 'edit',
    )
  }

  async sendMessage(
    taskId: string,
    message: string,
    resume = true,
    model?: string,
    fileIds?: string[],
    filePaths?: string[],
    commandName?: string,
    agentMode?: 'edit' | 'plan',
  ): Promise<void> {
    const metadata = await this.getTask(taskId)

    // Mark last iteration as willNotResume if resume is false
    if (!resume && metadata.iterations.length > 0) {
      const lastIteration = metadata.iterations[metadata.iterations.length - 1]
      if (!lastIteration.willNotResume) {
        lastIteration.willNotResume = true
        await this.storage.saveTask(metadata)

        logger.info('Marked last iteration as will not resume due to resume=false', {
          taskId: metadata.taskId,
          iterationNumber: lastIteration.iterationNumber,
        })
      }
    }

    // Create the iteration with 'created' status (this validates inputs)
    const iteration = await this.createTaskIteration(
      metadata,
      message,
      commandName,
      model,
      fileIds,
      filePaths,
      'created',
      agentMode,
    )

    // Check if command has willNotResume flag set
    if (commandName) {
      const command = await this.commandService.getCommand(commandName)
      if (command?.willNotResume) {
        iteration.willNotResume = true
        logger.info('Marked iteration as will not resume due to command flag', {
          taskId: metadata.taskId,
          commandName,
          iterationNumber: iteration.iterationNumber,
        })
      }
    }

    // Find the previous iteration's session ID for resume
    let resumeSessionId: string | undefined
    let prependedResult: string | undefined
    if (metadata.iterations.length > 0) {
      const previousIteration = metadata.iterations[metadata.iterations.length - 1]

      // Check if previous iteration has willNotResume flag set
      if (previousIteration.willNotResume && previousIteration.result) {
        // Prepend the result to the message instead of resuming
        prependedResult = previousIteration.result
        resumeSessionId = undefined
      } else if (resume) {
        // Normal resume behavior
        resumeSessionId = previousIteration.agentSessionId
      }
    }

    // Update metadata
    const now = new Date()
    metadata.iterations.push(iteration)
    metadata.lastMessageAt = now
    metadata.numTurns += 1

    // Save task with the new iteration in 'created' status
    await this.storage.saveTask(metadata)

    // Now start the iteration by updating its status to 'active' and starting execution
    iteration.status = 'active'

    // Save task with updated statuses
    await this.storage.saveTask(metadata)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'taskIteration',
      subType: 'active',
      payload: {
        status: 'active',
        previousStatus: 'created',
        iterationNumber: iteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: now,
      agentType: metadata.agentType,
      isError: false,
    })

    // Prepend result from last iteration if needed
    const finalMessage = prependedResult
      ? `Previous result:\n${prependedResult}\n\n${message}`
      : message

    // Execute agent through environment in background
    const agentRequest = {
      taskId,
      message: finalMessage,
      iteration,
      agentType: 'claude-code',
      resumeSessionId,
      model,
    }

    const backgroundTask = (async () => {
      for await (const event of this.agentService.executeAgent(agentRequest)) {
        // Emit agent events to event service
        await this.eventService.emitEvent(event)
      }
    })().catch(async (error) => {
      logger.error('Background agent execution failed', error as Error, { taskId })

      // Update status to error on agent failure
      try {
        // Only update iteration status, not task status
        iteration.status = 'failed'
        // Ensure we never set null for error messages
        const errorMessage = error instanceof Error ? error.message : String(error)
        iteration.result = `Background execution failed: ${errorMessage || 'Unknown error'}. Model: ${model || 'default'}, Resuming: ${!!resumeSessionId}`
        iteration.endTime = new Date()

        await this.storage.saveTask(metadata)

        // Emit error event
        await this.eventService.emitEvent({
          eventId: randomUUID(),
          taskId,
          type: 'taskIteration',
          subType: 'error',
          isError: true,
          agentType: metadata.agentType,
          timestamp: new Date(),
          payload: {
            status: 'error',
            previousStatus: 'active',
            iterationNumber: iteration.iterationNumber,
          },
        })
      } catch (saveError) {
        logger.error('Failed to update task status after agent error', saveError as Error)
      }
    })

    // Track the background task in WorkspaceService for status reporting
    this.workspaceService.trackBackgroundTask(taskId, backgroundTask)
  }

  async updateTask(taskId: string, updates: Partial<Task>): Promise<void> {
    const metadata = await this.getTask(taskId)

    const updatedMetadata = { ...metadata, ...updates }
    await this.storage.saveTask(updatedMetadata)
  }

  async updateTaskMetadata(
    taskId: string,
    updates: { name?: string; description?: string },
  ): Promise<void> {
    const task = await this.getTask(taskId)

    // Validate name format if provided
    if (updates.name !== undefined) {
      if (updates.name && !/^[a-zA-Z0-9-]+$/.test(updates.name)) {
        throw new ValidationError(
          'Task name must be URL-friendly (alphanumeric characters and dashes only)',
          { name: 'Invalid format' },
        )
      }
    }

    // Update only the provided fields
    if (updates.name !== undefined) {
      task.name = updates.name || undefined
    }
    if (updates.description !== undefined) {
      task.description = updates.description || undefined
    }

    await this.storage.saveTask(task)

    logger.info('Updated task metadata', {
      taskId,
      name: updates.name,
      description: updates.description ? `${updates.description.slice(0, 50)}...` : undefined,
    })
  }

  async deleteTask(taskId: string): Promise<void> {
    await this.getTask(taskId)

    // Clean up workspace (stops agents, servers, containers, etc.)
    await this.workspaceService.cleanupWorkspace(taskId)

    // Remove worktree since the task is being permanently deleted
    await this.workspaceService.removeWorktree(taskId)

    // Delete task data
    await this.storage.delete(`tasks/${taskId}`)
    await this.storage.delete(`events/${taskId}`)
  }

  async closeTask(taskId: string): Promise<void> {
    const task = await this.getTask(taskId)

    logger.debug('Closing task', { taskId })

    // Stop any running agent
    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace) {
        const environment = workspace.getEnvironment()
        await environment.stopAgent()
      }
    } catch (error) {
      // Log error but continue with cleanup
      logger.warn('Failed to stop agent during task close', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    // Stop preview server if running
    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace) {
        await workspace.stopServer()
      }
    } catch (error) {
      // Log error but continue with cleanup
      logger.warn('Failed to stop server during task close', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    // Clean up workspace (this will teardown the workspace)
    await this.workspaceService.cleanupWorkspace(taskId)

    // Remove worktree since the task is being closed
    await this.workspaceService.removeWorktree(taskId)

    // Update task status to closed
    const previousStatus = task.status
    task.status = 'closed'
    await this.storage.saveTask(task)

    // Emit status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'task',
      subType: 'closed',
      payload: {
        status: 'closed',
        previousStatus,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: task.agentType,
      isError: false,
    })

    logger.info('Task closed successfully', { taskId })
  }

  async getAvailableCommands(taskId: string): Promise<Command[]> {
    const task = await this.getTask(taskId)

    // Get all commands
    const allCommands = await this.commandService.listCommands()

    // Get workspace status to check for uncommitted changes
    const workspaceStatus = await this.workspaceService.getWorkspaceStatus(taskId)
    const gitStatus = (workspaceStatus as any).gitStatus
    const workspaceHasChanges = gitStatus?.hasUncommittedChanges || false

    // Get last iteration
    const lastIteration =
      task.iterations && task.iterations.length > 0
        ? task.iterations[task.iterations.length - 1]
        : undefined

    // Filter commands based on current context
    const context = {
      task,
      lastIteration,
      workspaceHasChanges,
      gitStatus,
    }

    return filterAvailableCommands(allCommands, context)
  }

  async markIterationAsWillNotResume(taskId: string, iterationNumber: number): Promise<void> {
    const task = await this.getTask(taskId)

    const iterationIndex = task.iterations.findIndex((it) => it.iterationNumber === iterationNumber)

    if (iterationIndex >= 0) {
      task.iterations[iterationIndex].willNotResume = true
      await this.storage.saveTask(task)

      logger.info('Marked iteration as will not resume', {
        taskId,
        iterationNumber,
      })
    } else {
      throw new ValidationError(
        `Iteration ${iterationNumber} not found for task ${taskId}`,
        'iterationNumber',
      )
    }
  }
}

export function createTaskService(
  storage: StorageBackend,
  eventService: EventService,
  workspaceService: WorkspaceService,
  commandService: CommandService,
  repositoryService: RepositoryService,
  agentService: AgentService,
): TaskService {
  return new TaskService(
    storage,
    eventService,
    workspaceService,
    commandService,
    repositoryService,
    agentService,
  )
}
