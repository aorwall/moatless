import { randomUUID } from 'node:crypto'
import type {
  Command,
  CreateTaskRequest,
  CreateVibeTaskRequest,
  Event,
  MessageContext,
  SDKResultMessage,
  Task,
  TaskEvent,
  TaskIteration,
  TaskIterationEvent,
} from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, singleton } from 'tsyringe'
import type { StorageBackend } from '../storage/index.js'
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
import type { Workspace } from '../workspaces/workspace.js'
import type { AgentService } from './agent.service.js'
import type { CommandService } from './command.service.js'
import type { EventService } from './event.service.js'
import type { MessageService } from './message.service.js'
import type { RepositoryService } from './repository.service.js'
import type { ScreenshotService } from './screenshot.service.js'
import type { WorkspaceService } from './workspace.service.js'

@singleton()
export class TaskService {
  private static readonly WORKSPACE_READY_TIMEOUT = 30000
  private static readonly WORKSPACE_CHECK_INTERVAL = 500

  constructor(
    @inject('Storage') private storage: StorageBackend,
    private eventService: EventService,
    private workspaceService: WorkspaceService,
    private commandService: CommandService,
    private repositoryService: RepositoryService,
    private agentService: AgentService,
    private screenshotService: ScreenshotService,
    private messageService: MessageService,
  ) {
    // Subscribe to agent events for reactive task updates
    this.setupEventSubscription()
  }

  /**
   * Derive task status based on the last iteration's status
   * If the last iteration is "active", the task is considered "running"
   * If the task is "running" but iteration is no longer active, derive back to "open" or "error"
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
    if (lastIteration && lastIteration.status === 'active') {
      return {
        ...task,
        status: 'running',
      }
    }

    // If task status is 'running' but last iteration is not active,
    // derive status back to 'open' (or 'error' if iteration failed)
    // This handles cases where the task was saved with 'running' status
    // but the iteration has since completed/failed
    if (task.status === 'running') {
      const newStatus = lastIteration?.status === 'failed' ? 'error' : 'open'
      return {
        ...task,
        status: newStatus,
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

  private findIterationForEvent(task: Task, event: Event): TaskIteration | undefined {
    const hasIterationId =
      (event.type === 'agent' || event.type === 'taskIteration') && 'iterationId' in event

    if (hasIterationId) {
      return task.iterations.find((it) => it.iterationNumber === event.iterationId)
    }

    return task.iterations[task.iterations.length - 1]
  }

  private async handleAgentEventByType(
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

      // Process the event and save if handled
      const wasHandled = await this.handleAgentEventByType(task, iteration, event)
      if (!wasHandled && agentSessionId) {
        await this.storage.saveTask(task)
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

    // Determine iteration status and result based on payload
    this.applyResultToIteration(iteration, event, resultPayload, taskLogger)

    iteration.endTime = new Date()

    // Perform post-processing steps
    await this.performIterationPostProcessing(task, iteration, taskLogger)

    // Update and save task
    const previousStatus = task.status
    task.status = iteration.status === 'failed' ? 'error' : 'open'
    await this.storage.saveTask(task)

    // Emit appropriate events
    await this.emitIterationEvents(task, iteration, previousStatus)
  }

  private applyResultToIteration(
    iteration: TaskIteration,
    event: Event,
    resultPayload: SDKResultMessage,
    taskLogger: ReturnType<typeof logger.child>,
  ): void {
    if (event.isError || resultPayload.is_error) {
      this.applyErrorResult(iteration, resultPayload)
    } else if (resultPayload.subtype === 'success') {
      this.applySuccessResult(iteration, resultPayload, taskLogger)
    } else {
      this.applyUnexpectedResult(iteration, resultPayload, taskLogger)
    }
  }

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

  private async performIterationPostProcessing(
    task: Task,
    iteration: TaskIteration,
    taskLogger: ReturnType<typeof logger.child>,
  ): Promise<void> {
    await this.captureGitDataForIteration(task.taskId, iteration)

    if (task.gitCheckpointEnabled) {
      await this.createIterationCheckpoint(task, iteration)
    }

    if (task.gitPushEnabled) {
      await this.pushIterationChanges(task, iteration)
    }

    await this.captureCommitHashIfNeeded(task.taskId, iteration, taskLogger)

    this.screenshotService
      .captureIterationScreenshotIfEnabled(task.taskId, iteration)
      .catch((error) => {
        taskLogger.warn('Failed to capture preview screenshot', {
          error: error instanceof Error ? error.message : String(error),
          iterationNumber: iteration.iterationNumber,
        })
      })
  }

  private async captureCommitHashIfNeeded(
    taskId: string,
    iteration: TaskIteration,
    taskLogger: ReturnType<typeof logger.child>,
  ): Promise<void> {
    if (iteration.checkpointCommitHash) {
      return
    }

    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (!workspace) {
        return
      }

      const workspacePath = workspace.getWorkspacePath()
      const commitHash = await getCurrentCommitHash(workspacePath)
      if (commitHash) {
        iteration.checkpointCommitHash = commitHash
      }
    } catch (error) {
      taskLogger.debug('Could not capture commit hash', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  private async emitIterationEvents(
    task: Task,
    iteration: TaskIteration,
    previousStatus: string,
  ): Promise<void> {
    if (iteration.status === 'completed') {
      await this.emitCompletedIterationEvent(task, iteration)
    }

    if (previousStatus !== task.status) {
      await this.emitTaskStatusChangeEvent(task, previousStatus)
    }
  }

  private async emitCompletedIterationEvent(task: Task, iteration: TaskIteration): Promise<void> {
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

  private async emitTaskStatusChangeEvent(task: Task, previousStatus: string): Promise<void> {
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'task',
      subType: task.status as 'open' | 'closed',
      agentType: task.agentType,
      payload: {
        status: task.status,
        previousStatus,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: task.status === 'error',
    } as TaskEvent)
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
    iteration.result = `${(event.payload?.error as string) || 'Agent execution failed'}${
      event.payload?.stackTrace ? `\nStack: ${event.payload.stackTrace}` : ''
    }`
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
   * Handle MESSAGE events that contain context token updates
   */
  private async handleMessageEvent(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<void> {
    // Update contextTokens if provided in the event payload (for AgentEvents)
    const contextTokens =
      event.type === 'agent' && typeof event.payload?.contextTokens === 'number'
        ? event.payload.contextTokens
        : undefined
    if (contextTokens !== undefined) {
      iteration.contextTokens = contextTokens

      // Save task with updated context tokens
      await this.storage.saveTask(task)

      logger.debug('Updated context tokens from MESSAGE event', {
        taskId: task.taskId,
        iterationNumber: iteration.iterationNumber,
        contextTokens,
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
    const taskLogger = logger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

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
      hasBranchName: Boolean(params.branchName),
      hasMessage: Boolean(params.message),
      hasFileIds: (params.fileIds?.length ?? 0) > 0,
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
    const task = this.initializeTaskMetadata(params, now)

    // Create task iteration with 'created' status if message or contexts are provided
    if (params.message || (params.contexts && params.contexts.length > 0)) {
      const iteration = this.createTaskIteration(
        task,
        params.message,
        undefined,
        params.model,
        'created',
        params.agentMode, // agentMode from create task request
        params.contexts, // contexts from create task request
      )
      task.iterations.push(iteration)
    }

    await this.persistTaskAndEmitCreatedEvent(task, params, now)
    await this.createWorkspaceForTask(task, params)

    logger.info('Task creation completed successfully', {
      taskId: task.taskId,
    })

    await this.handleTaskStartOrSetup(task, params)

    return task
  }

  /**
   * Initialize task metadata object with all required fields
   */
  private initializeTaskMetadata(params: CreateTaskRequest, now: Date): Task {
    return {
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
  }

  /**
   * Persist task to storage and emit task created event
   */
  private async persistTaskAndEmitCreatedEvent(
    task: Task,
    params: CreateTaskRequest,
    now: Date,
  ): Promise<void> {
    try {
      await this.storage.saveTask(task)

      // Emit task created event
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'task',
        subType: 'open',
        agentType: params.agentType,
        payload: {
          status: 'open',
          previousStatus: 'created',
        },
        eventId: randomUUID(),
        timestamp: now,
        isError: false,
      })

      logger.info('Task metadata created successfully', {
        taskId: task.taskId,
        agentType: params.agentType,
        repositoryId: params.repositoryId,
        hasInitialIteration: Boolean(params.message),
      })
    } catch (error) {
      logger.error('Failed to save task metadata', error as Error)
      throw error
    }
  }

  /**
   * Create workspace for task and update task with workspace path
   */
  private async createWorkspaceForTask(task: Task, params: CreateTaskRequest): Promise<void> {
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
  }

  /**
   * Handle task start or background setup based on params
   * Saves initial message if starting, then either waits for agent start or sets up workspace in background
   */
  private async handleTaskStartOrSetup(task: Task, params: CreateTaskRequest): Promise<void> {
    // Determine if task should start immediately
    const shouldStart =
      (params.start ?? true) &&
      (Boolean(params.message) || (params.contexts && params.contexts.length > 0))

    if (shouldStart) {
      // Save initial user message before starting agent
      const firstIteration = task.iterations[0]
      if (firstIteration) {
        await this.messageService.saveUserMessage(
          task.taskId,
          params.message || '',
          undefined, // no resumeSessionId for first iteration
          firstIteration.iterationNumber,
          params.fileIds,
          params.contexts,
        )
        logger.debug('Initial user message saved during task creation', {
          taskId: task.taskId,
          iterationNumber: firstIteration.iterationNumber,
          hasAttachments: (params.fileIds?.length ?? 0) > 0,
          hasContexts: (params.contexts?.length ?? 0) > 0,
        })
      }

      // Synchronous setup: wait for agent to start and first event to be emitted
      await this.setupWorkspaceAndWaitForAgentStart(
        task.taskId,
        params.start,
        params.message,
        params.fileIds,
        params.contexts,
      )
    } else {
      // Background workspace setup only (no agent start)
      this.workspaceService.setupWorkspace(task.taskId).catch((error) => {
        logger.error('Background workspace setup failed', error as Error, {
          taskId: task.taskId,
        })
      })
    }
  }

  /**
   * Sets up workspace and waits for agent to start (first event)
   * Used for synchronous task creation to ensure message is saved before returning
   */
  private async setupWorkspaceAndWaitForAgentStart(
    taskId: string,
    start?: boolean,
    message?: string,
    fileIds?: string[],
    contexts?: MessageContext[],
  ): Promise<void> {
    logger.info('Setting up workspace and waiting for agent start', { taskId })

    // Setup workspace (this is synchronous)
    await this.workspaceService.setupWorkspace(taskId)

    // Check if workspace setup succeeded
    const workspace = await this.workspaceService.getWorkspace(taskId)
    if (workspace && workspace.getStatus() === 'error') {
      await this.handleWorkspaceSetupError(taskId, workspace)
      throw new Error('Workspace setup failed')
    }

    // Start task with waitForFirstEvent=true
    const shouldStart = (start ?? true) && (Boolean(message) || (contexts && contexts.length > 0))
    if (shouldStart) {
      await this.startTaskAndWaitForFirstEvent(taskId, fileIds, contexts)
    }

    logger.info('Workspace setup and agent start completed', {
      taskId,
      started: shouldStart,
    })
  }

  /**
   * Helper method to start task and wait for first event
   * Wraps startTask() with waitForFirstEvent=true
   */
  private async startTaskAndWaitForFirstEvent(
    taskId: string,
    fileIds?: string[],
    contexts?: MessageContext[],
  ): Promise<void> {
    await this.startTask(taskId, fileIds, contexts, true)
  }

  private async handleWorkspaceSetupError(taskId: string, workspace: Workspace): Promise<void> {
    const task = await this.storage.getTask(taskId)
    if (!task || task.iterations.length === 0) {
      logger.error('Workspace setup failed, skipping task start', { taskId })
      return
    }

    const lastIteration = task.iterations[task.iterations.length - 1]
    if (!lastIteration || lastIteration.status !== 'created') {
      logger.error('Workspace setup failed, skipping task start', { taskId })
      return
    }

    const environment = workspace.getEnvironment()
    const errorMessage = environment.getError() || 'Environment setup failed'

    lastIteration.status = 'failed'
    lastIteration.result = `Environment setup failed: ${errorMessage}`
    lastIteration.endTime = new Date()

    await this.storage.saveTask(task)
    await this.emitIterationErrorEvent(taskId, task.agentType, lastIteration, 'created')

    logger.error('Workspace setup failed, skipping task start', { taskId })
  }

  private async emitIterationErrorEvent(
    taskId: string,
    agentType: string,
    iteration: TaskIteration,
    previousStatus: string,
  ): Promise<void> {
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      taskId,
      type: 'taskIteration',
      subType: 'error',
      agentType,
      iterationId: iteration.iterationNumber,
      timestamp: new Date(),
      isError: true,
      payload: {
        status: 'error',
        previousStatus,
        iterationNumber: iteration.iterationNumber,
      },
    })
  }

  /**
   * Starts a task by activating the last iteration with 'created' status
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Pre-existing complexity, refactoring planned
  async startTask(
    taskId: string,
    fileIds?: string[],
    contexts?: MessageContext[],
    waitForFirstEvent = false,
  ): Promise<void> {
    logger.info('Starting task', {
      taskId,
      hasFileIds: (fileIds?.length ?? 0) > 0,
      hasContexts: (contexts?.length ?? 0) > 0,
    })

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
      agentType: task.agentType,
      iterationId: lastIteration.iterationNumber,
      payload: {
        status: 'active',
        previousStatus: lastIteration.status,
        iterationNumber: lastIteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: false,
    })

    // Execute the agent with the iteration's initial message
    if (lastIteration.initialMessage || lastIteration.commandName) {
      // Fetch repository to get system prompt
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)

      const agentRequest: {
        taskId: string
        message: string
        iteration: typeof lastIteration
        agentType: string
        resumeSessionId?: string
        model?: string
        systemPrompt?: string
        fileIds?: string[]
        contexts?: MessageContext[]
      } = {
        taskId,
        message: lastIteration.initialMessage || '',
        iteration: lastIteration,
        agentType: 'claude-code',
        ...(lastIteration.model !== undefined && {
          model: lastIteration.model,
        }),
        ...(repository?.systemPrompt !== undefined && {
          systemPrompt: repository.systemPrompt,
        }),
        ...(fileIds !== undefined && fileIds.length > 0 && { fileIds }),
        ...(contexts !== undefined && contexts.length > 0 && { contexts }),
      }

      // Get the agent event generator
      const eventGenerator = this.agentService.executeAgent(agentRequest)

      // Wait for first event if requested (for synchronous task creation)
      if (waitForFirstEvent) {
        const firstEventTimeout = 30000 // 30 seconds
        const firstEventPromise = eventGenerator.next()
        const timeoutPromise = new Promise<never>((_, reject) =>
          setTimeout(
            () => reject(new Error('Agent start timeout - first event not received within 30s')),
            firstEventTimeout,
          ),
        )

        try {
          const firstResult = await Promise.race([firstEventPromise, timeoutPromise])

          if (!firstResult.done && firstResult.value) {
            // Emit the first event
            await this.eventService.emitEvent(firstResult.value)
            logger.info('First agent event received and emitted', {
              taskId,
              eventType: firstResult.value.type,
              eventSubType: 'subType' in firstResult.value ? firstResult.value.subType : undefined,
            })
          } else if (firstResult.done) {
            throw new Error('Agent completed without emitting any events')
          }
        } catch (error) {
          logger.error('Failed to receive first agent event', error as Error, {
            taskId,
            iterationNumber: lastIteration.iterationNumber,
          })

          // Update iteration status to failed
          const updatedTask = await this.storage.getTask(taskId)
          if (updatedTask) {
            const updatedIteration = updatedTask.iterations.find(
              (it) => it.iterationNumber === lastIteration.iterationNumber,
            )
            if (updatedIteration) {
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
                agentType: updatedTask.agentType,
                iterationId: lastIteration.iterationNumber,
                timestamp: new Date(),
                isError: true,
                payload: {
                  status: 'error',
                  previousStatus: 'active',
                  iterationNumber: lastIteration.iterationNumber,
                },
              })
            }
          }

          throw error
        }
      }

      // Process remaining events in background
      const executeTask = (async () => {
        for await (const event of eventGenerator) {
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
                agentType: updatedTask.agentType,
                iterationId: lastIteration.iterationNumber,
                timestamp: new Date(),
                isError: true,
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
      hasMessage: Boolean(params.message),
      model: params.model,
      hasFileIds: (params.fileIds?.length ?? 0) > 0,
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

    // Create a new repository from the template
    const repository = await this.repositoryService.createRepositoryFromTemplate(
      repositoryName,
      params.templateId,
    )

    // Create a task with the new repository
    // Vibe tasks always start immediately
    const createTaskParams: CreateTaskRequest = {
      agentType: params.agentType,
      repositoryId: repository.id,
      message: params.message,
      start: params.start,
      model: params.model,
      fileIds: params.fileIds, // Support file uploads for context
      contexts: params.contexts, // Support context items (e.g., Figma imports)
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

    // Derive task status
    const taskWithStatus = this.deriveTaskStatus(task)

    // Fetch repository for display name
    const repository = await this.repositoryService.getRepositoryById(task.repositoryId)

    // Compute title from name, first message, or agent type
    const title =
      task.name ||
      (task.iterations?.[0]?.initialMessage?.split('\n')[0] ?? task.agentType) ||
      'Task'

    // Add repository name
    const repositoryName = repository?.name || task.repositoryId

    return {
      ...taskWithStatus,
      title,
      repositoryName,
    }
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
        } else if (taskWithDerivedStatus.status !== 'closed') {
          // If no status filter, exclude closed tasks by default
          tasks.push(taskWithDerivedStatus)
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
  private createTaskIteration(
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

  async sendCommand(
    taskId: string,
    commandName: string,
    resume = true,
    model?: string,
    fileIds?: string[],
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
      fileIds,
      commandName,
      command.agentMode || 'edit',
    )
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Refactoring planned in future iteration
  async sendMessage(
    taskId: string,
    message: string,
    resume = true,
    model?: string,
    fileIds?: string[],
    commandName?: string,
    agentMode?: 'edit' | 'plan',
    contexts?: MessageContext[],
  ): Promise<void> {
    const metadata = await this.getTask(taskId)

    // Mark last iteration as willNotResume if resume is false
    if (!resume && metadata.iterations.length > 0) {
      const lastIteration = metadata.iterations[metadata.iterations.length - 1]
      if (lastIteration && !lastIteration.willNotResume) {
        lastIteration.willNotResume = true
        await this.storage.saveTask(metadata)

        logger.info('Marked last iteration as will not resume due to resume=false', {
          taskId: metadata.taskId,
          iterationNumber: lastIteration.iterationNumber,
        })
      }
    }

    // Create the iteration with 'created' status (this validates inputs)
    const iteration = this.createTaskIteration(
      metadata,
      message,
      commandName,
      model,
      'created',
      agentMode,
      contexts,
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
      if (previousIteration?.willNotResume && previousIteration.result) {
        // Prepend the result to the message instead of resuming
        prependedResult = previousIteration.result
        resumeSessionId = undefined
      } else if (resume && previousIteration) {
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
      agentType: metadata.agentType,
      iterationId: iteration.iterationNumber,
      payload: {
        status: 'active',
        previousStatus: 'created',
        iterationNumber: iteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: now,
      isError: false,
    })

    // Prepend result from last iteration if needed
    const finalMessage = prependedResult
      ? `Previous result:\n${prependedResult}\n\n${message}`
      : message

    // Fetch repository to get system prompt
    const repository = await this.repositoryService.getRepositoryById(metadata.repositoryId)

    // Execute agent through environment in background
    const agentRequest: {
      taskId: string
      message: string
      iteration: typeof iteration
      agentType: string
      resumeSessionId?: string
      model?: string
      systemPrompt?: string
      fileIds?: string[]
      contexts?: MessageContext[]
    } = {
      taskId,
      message: finalMessage,
      iteration,
      agentType: 'claude-code',
      ...(resumeSessionId !== undefined && { resumeSessionId }),
      ...(model !== undefined && { model }),
      ...(repository?.systemPrompt !== undefined && {
        systemPrompt: repository.systemPrompt,
      }),
      ...(fileIds !== undefined && fileIds.length > 0 && { fileIds }),
      ...(contexts !== undefined && contexts.length > 0 && { contexts }),
    }

    // Save the initial user message before starting agent
    await this.messageService.saveUserMessage(
      taskId,
      message,
      resumeSessionId,
      iteration.iterationNumber,
      fileIds,
      contexts,
    )

    logger.debug('Initial user message saved during sendMessage', {
      taskId,
      iterationNumber: iteration.iterationNumber,
      hasSessionId: Boolean(resumeSessionId),
      hasAttachments: (fileIds?.length ?? 0) > 0,
      hasContexts: (contexts?.length ?? 0) > 0,
    })

    const backgroundTask = (async () => {
      for await (const event of this.agentService.executeAgent(agentRequest)) {
        // Emit agent events to event service
        await this.eventService.emitEvent(event)
      }
    })().catch(async (error) => {
      logger.error('Background agent execution failed', error as Error, {
        taskId,
      })

      // Update status to error on agent failure
      try {
        // Only update iteration status, not task status
        iteration.status = 'failed'
        // Ensure we never set null for error messages
        const errorMessage = error instanceof Error ? error.message : String(error)
        iteration.result = `Background execution failed: ${errorMessage || 'Unknown error'}. Model: ${model || 'default'}, Resuming: ${Boolean(resumeSessionId)}`
        iteration.endTime = new Date()

        await this.storage.saveTask(metadata)

        // Emit error event
        await this.eventService.emitEvent({
          eventId: randomUUID(),
          taskId,
          type: 'taskIteration',
          subType: 'error',
          agentType: metadata.agentType,
          iterationId: iteration.iterationNumber,
          timestamp: new Date(),
          isError: true,
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
    if (updates.name !== undefined && updates.name && !/^[a-zA-Z0-9-]+$/.test(updates.name)) {
      throw new ValidationError(
        'Task name must be URL-friendly (alphanumeric characters and dashes only)',
        { name: 'Invalid format' },
      )
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
      agentType: task.agentType,
      payload: {
        status: 'closed',
        previousStatus,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
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
    const { gitStatus } = workspaceStatus
    const workspaceHasChanges = gitStatus?.hasUncommittedChanges

    // Get last iteration
    const lastIteration =
      task.iterations && task.iterations.length > 0
        ? task.iterations[task.iterations.length - 1]
        : undefined

    // Filter commands based on current context
    const context = {
      ...(task !== undefined && { task }),
      ...(lastIteration !== undefined && { lastIteration }),
      ...(workspaceHasChanges !== undefined && { workspaceHasChanges }),
      ...(gitStatus !== undefined && { gitStatus }),
    }

    return filterAvailableCommands(allCommands, context)
  }
}

/**
 * @deprecated Use TSyringe container.resolve(TaskService) instead
 * Temporary factory function for backward compatibility during migration
 */
export function createTaskService(
  storage: StorageBackend,
  eventService: EventService,
  workspaceService: WorkspaceService,
  commandService: CommandService,
  repositoryService: RepositoryService,
  agentService: AgentService,
  screenshotService: ScreenshotService,
  messageService: MessageService,
): TaskService {
  return new TaskService(
    storage,
    eventService,
    workspaceService,
    commandService,
    repositoryService,
    agentService,
    screenshotService,
    messageService,
  )
}
