import { randomUUID } from 'node:crypto'
import type { CreateTaskRequest, Event, MessageContext, Task, TaskIteration } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { AgentRunOptions } from '../../agents/agent/base-agent'
import type { AgentController } from '../../agents/services/agent-controller.service'
import { CommandService } from '../../agents/services/command.service'
import { MessageService } from '../../agents/services/message.service'
import { AgentControllerSymbol } from '../../agents/symbols'
import { runInBackgroundWithSession } from '../../auth'
import { EventService } from '../../events/index'
import { RepositoryService } from '../../repositories/index'
import { WorkspaceCoordinator } from '../../workspace/services/workspace-coordinator.service'
import { TaskRepository } from '../db/task.repository'
import { TaskIterationService } from './task-iteration.service'

/**
 * Agent request parameters for background execution
 */
interface AgentRequest {
  taskId: string
  message: string
  iteration: TaskIteration
  agentType: string
  resumeSessionId?: string
  model?: string
  systemPrompt?: string
  fileIds?: string[]
  contexts?: MessageContext[]
}

/**
 * TaskExecutionCoordinator - Orchestrates agent execution and workspace management
 *
 * Handles:
 * - Starting tasks and iterations
 * - Sending messages and commands
 * - Workspace provisioning and validation
 * - Agent execution in background with error handling
 * - Resume logic for continuing iterations
 * - Auto-resume of unfinished iterations when workspace becomes ready
 *
 * This is a complex coordinator that brings together multiple services to execute
 * agent workflows end-to-end.
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskExecutionCoordinator {
  private static readonly WORKSPACE_READY_TIMEOUT = 30000 // 30 seconds
  private static readonly WORKSPACE_CHECK_INTERVAL = 500 // 500ms

  private unsubscribe?: (() => void) | undefined

  constructor(
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(TaskIterationService) private readonly iterationService: TaskIterationService,
    @inject(WorkspaceCoordinator) private readonly workspaceCoordinator: WorkspaceCoordinator,
    @inject(AgentControllerSymbol) private readonly agentController: AgentController,
    @inject(EventService) private readonly eventService: EventService,
    @inject(RepositoryService) private readonly repositoryService: RepositoryService,
    @inject(CommandService) private readonly commandService: CommandService,
    @inject(MessageService) private readonly messageService: MessageService,
  ) {}

  /**
   * Start listening for environment:ready events from EventService.
   * Called once during app initialization.
   * This enables automatic resume of unfinished iterations when workspace becomes ready.
   */
  startEventSubscription(): void {
    if (this.unsubscribe) {
      logger.warn('TaskExecutionCoordinator: Event subscription already started')
      return
    }

    this.unsubscribe = this.eventService.subscribeAll(async (event) => {
      if (event.type === 'environment' && event.subType === 'ready') {
        try {
          await this.handleEnvironmentReadyEvent(event)
        } catch (error) {
          logger.warn(
            {
              taskId: event.taskId,
              eventId: event.eventId,
              error: error instanceof Error ? error.message : String(error),
            },
            'Failed to handle environment:ready event for auto-resume',
          )
        }
      }
    })
    logger.info('TaskExecutionCoordinator: Started event subscription for auto-resume')
  }

  /**
   * Stop event subscription (for graceful shutdown)
   */
  stopEventSubscription(): void {
    if (this.unsubscribe) {
      this.unsubscribe()
      this.unsubscribe = undefined
      logger.info('TaskExecutionCoordinator: Stopped event subscription')
    }
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
        await this.resumeIteration(taskId, unfinishedIteration)
      } else if (unfinishedIteration.status === 'created') {
        // Start the created iteration
        await this.startTask(taskId)
      }
    } catch (error) {
      taskLogger.error(
        { error: error instanceof Error ? error.message : String(error) },
        'Failed to auto-resume iteration after workspace ready',
      )
    }
  }

  /**
   * Resume an existing active iteration without creating a new one.
   * Used for auto-recovery after workspace restart.
   *
   * Behavior:
   * - WITH agentSessionId: Resume using SDK session resume feature (sends empty message)
   * - WITHOUT agentSessionId: Start fresh using iteration.initialMessage (can't resume SDK session)
   *
   * @param taskId - Task ID
   * @param iteration - The active iteration to resume
   */
  async resumeIteration(taskId: string, iteration: TaskIteration): Promise<void> {
    const taskLogger = logger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
      handler: 'resumeIteration',
    })

    // Validate iteration is active
    if (iteration.status !== 'active') {
      throw new ValidationError(
        `Cannot resume iteration with status "${iteration.status}". Only "active" iterations can be resumed.`,
        { status: iteration.status },
      )
    }

    // Check if agent is already running
    const handle = await this.workspaceCoordinator.ensureWorkspace(taskId)
    if (handle.agent.state === 'running') {
      const actuallyRunning = await this.workspaceCoordinator.isAgentActuallyRunning(taskId)
      if (actuallyRunning) {
        taskLogger.warn('Agent is already running, skipping resume')
        return
      }
      // State was stale - reset it
      taskLogger.warn('Agent state was running but worker says idle, resetting stale state')
      await this.workspaceCoordinator.updateAgentState(taskId, { state: 'idle' })
    }

    // Wait for workspace to be ready
    await this.waitForWorkspaceReady(taskId)

    // Get task for repository ID
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Determine message and resume strategy
    let message: string
    let resumeSessionId: string | undefined

    if (iteration.agentSessionId) {
      // Resume with SDK session - send empty message to continue
      message = ''
      resumeSessionId = iteration.agentSessionId
      taskLogger.info(
        { agentSessionId: iteration.agentSessionId },
        'Resuming with SDK session resume',
      )
    } else {
      // No session ID - start fresh using initialMessage
      message = iteration.initialMessage || ''
      if (!message) {
        taskLogger.error('Cannot resume iteration without agentSessionId or initialMessage')
        await this.iterationService.markIterationAsFailed(
          taskId,
          'Cannot resume: no agent session ID or initial message available',
          task,
        )
        return
      }
      taskLogger.info('Starting fresh on existing iteration (no session ID to resume)')
    }

    // Build run options
    const runOptions: AgentRunOptions = {
      iterationNumber: iteration.iterationNumber,
      userMessageSaved: true, // Don't save user message again
      ...(resumeSessionId !== undefined && { resume: resumeSessionId }),
      ...(iteration.model !== undefined && { model: iteration.model }),
      ...(iteration.agentMode !== undefined && { agentMode: iteration.agentMode }),
      ...(iteration.background !== undefined && { background: iteration.background }),
    }

    taskLogger.info({ runOptions }, 'Executing agent for resumed iteration')

    // Execute agent in background
    await this.agentController.runAgentInBackground(
      taskId,
      'claude-code',
      message,
      runOptions,
      iteration,
      task.repositoryId,
      task,
    )
  }

  /**
   * Send a command to a task (command prompt becomes the message).
   *
   * @param taskId - Task ID
   * @param commandName - Name of command to execute
   * @param resume - Whether to resume from previous iteration (default: true)
   * @param model - Optional model override
   * @param fileIds - Optional file attachments
   */
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

  /**
   * Send a message to a task, creating a new iteration and executing the agent.
   *
   * This is the main entry point for agent execution. It:
   * 1. Loads the task
   * 2. Marks previous iteration as will-not-resume if needed
   * 3. Creates a new iteration
   * 4. Determines resume session ID
   * 5. Saves the task and emits events
   * 6. Saves user message
   * 7. Executes agent in background
   *
   * @param taskId - Task ID
   * @param message - User message
   * @param resume - Whether to resume from previous iteration
   * @param model - Optional model override
   * @param fileIds - Optional file attachments
   * @param commandName - Optional command name
   * @param agentMode - Agent mode (edit/plan)
   * @param contexts - Optional message contexts
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex orchestration logic necessary for agent execution
  async sendMessage(
    taskId: string,
    message: string,
    resume = true,
    model?: string,
    fileIds?: string[],
    commandName?: string,
    agentMode?: 'edit' | 'plan',
    contexts?: MessageContext[],
    background?: boolean,
  ): Promise<void> {
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Save user message FIRST (before workspace operations) to ensure it's persisted
    // even if workspace provisioning takes time or fails
    const iterationNumber = task.iterations.length + 1
    await this.messageService.saveUserMessage(
      taskId,
      message,
      undefined, // sessionId - not known yet
      iterationNumber,
      fileIds,
      contexts,
    )
    logger.info(
      { taskId, iterationNumber, hasContexts: (contexts?.length ?? 0) > 0 },
      'User message saved before workspace operations',
    )

    // Determine effective background flag (per-message override > task-level)
    const effectiveBackground = background ?? task.background

    // Check if an agent is already running for this task
    // This prevents the fire-and-forget background execution from failing silently
    const handle = await this.workspaceCoordinator.ensureWorkspace(taskId, task.repositoryId)
    if (handle.agent.state === 'running') {
      // Validate with the actual worker to avoid false positives from stale state
      const actuallyRunning = await this.workspaceCoordinator.isAgentActuallyRunning(taskId)

      if (actuallyRunning) {
        logger.warn({ taskId }, 'Attempted to send message while agent is running')
        throw new HTTPException(409, { message: 'Agent is already running for this task' })
      }

      // State was stale - reset it and continue
      logger.warn({ taskId }, 'Agent state was running but worker says idle, resetting stale state')
      await this.workspaceCoordinator.updateAgentState(taskId, { state: 'idle' })
    }

    // Mark last iteration as willNotResume if resume is false
    if (!resume && task.iterations.length > 0) {
      const lastIteration = task.iterations[task.iterations.length - 1]
      if (lastIteration && !lastIteration.willNotResume) {
        lastIteration.willNotResume = true
        await this.taskRepository.save(task)

        logger.info(
          {
            taskId: task.taskId,
            iterationNumber: lastIteration.iterationNumber,
          },
          'Marked last iteration as will not resume due to resume=false',
        )
      }
    }

    // Create the iteration with 'created' status
    const iteration = this.iterationService.createIteration(
      task,
      message,
      commandName,
      model,
      'created',
      agentMode,
      contexts,
      effectiveBackground,
    )

    // Check if command has willNotResume flag set
    if (commandName) {
      const command = await this.commandService.getCommand(commandName)
      if (command?.willNotResume) {
        iteration.willNotResume = true
        logger.info(
          {
            taskId: task.taskId,
            commandName,
            iterationNumber: iteration.iterationNumber,
          },
          'Marked iteration as will not resume due to command flag',
        )
      }
    }

    // Find the previous iteration's session ID for resume
    let resumeSessionId: string | undefined
    let prependedResult: string | undefined
    if (task.iterations.length > 0) {
      const previousIteration = task.iterations[task.iterations.length - 1]

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
    task.iterations.push(iteration)
    task.lastMessageAt = now
    task.numTurns += 1

    // Save task with the new iteration in 'created' status
    await this.taskRepository.save(task)

    // Now start the iteration by updating its status to 'active' and starting execution
    iteration.status = 'active'

    // Save task with updated statuses
    await this.taskRepository.save(task)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'taskIteration',
      subType: 'active',
      agentType: task.agentType,
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
    const repository = await this.repositoryService.getRepositoryById(task.repositoryId)

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

    // Execute agent in background
    // AgentController.startAgent saves the user message synchronously before spawning background work
    await this.executeAgentInBackground(taskId, agentRequest, iteration, task)
  }

  /**
   * Start a task by activating the last 'created' iteration.
   *
   * This method:
   * 1. Validates last iteration is in 'created' status
   * 2. Waits for workspace to be ready
   * 3. Updates iteration to 'active' and emits event
   * 4. Executes the agent in background
   *
   * @param taskId - Task ID to start
   * @param fileIds - Optional file attachments
   * @param contexts - Optional message contexts
   * @param _waitForFirstEvent - Deprecated, no longer used (events come via Redis subscription)
   */
  async startTask(
    taskId: string,
    fileIds?: string[],
    contexts?: MessageContext[],
    _waitForFirstEvent = false,
  ): Promise<void> {
    logger.info(
      {
        taskId,
        hasFileIds: (fileIds?.length ?? 0) > 0,
        hasContexts: (contexts?.length ?? 0) > 0,
      },
      'Starting task',
    )

    const task = await this.taskRepository.get(taskId)
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

    // Wait for workspace to be ready by checking snapshot
    await this.waitForWorkspaceReady(taskId)

    // Update iteration status to active
    lastIteration.status = 'active'

    // Save task with updated statuses
    await this.taskRepository.save(task)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'taskIteration',
      subType: 'active',
      agentType: task.agentType,
      iterationId: lastIteration.iterationNumber,
      payload: {
        status: 'active',
        previousStatus: 'created',
        iterationNumber: lastIteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: false,
    })

    // Execute the agent with the iteration's initial message
    if (lastIteration.initialMessage || lastIteration.commandName) {
      // Build run options for AgentController
      const runOptions: AgentRunOptions = {
        iterationNumber: lastIteration.iterationNumber,
        ...(lastIteration.model !== undefined && { model: lastIteration.model }),
        ...(lastIteration.agentMode !== undefined && { agentMode: lastIteration.agentMode }),
        ...(fileIds !== undefined && fileIds.length > 0 && { fileIds }),
        ...(contexts !== undefined && contexts.length > 0 && { contexts }),
        ...(lastIteration.background !== undefined && { background: lastIteration.background }),
      }

      // Execute agent in background (events handled by TaskIterationService subscription)
      await this.agentController.runAgentInBackground(
        taskId,
        'claude-code',
        lastIteration.initialMessage || '',
        runOptions,
        lastIteration,
        task.repositoryId,
        task,
      )
    }

    logger.debug(
      {
        taskId,
        iterationNumber: lastIteration.iterationNumber,
      },
      'Task started successfully',
    )
  }

  /**
   * Create and setup workspace for a task, updating task with workspace path.
   *
   * @param task - Task to setup workspace for
   * @param repositoryId - Repository ID for the workspace
   */
  async createAndSetupWorkspace(task: Task, repositoryId: string): Promise<void> {
    try {
      const handle = await this.workspaceCoordinator.ensureWorkspace(task.taskId, repositoryId)
      task.workspacePath = handle.workspacePath

      // Save task with workspace path
      await this.taskRepository.save(task)
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: task.taskId, repositoryId },
        'Failed to create workspace',
      )
      throw error
    }
  }

  /**
   * Handle task start or background setup based on params.
   * Saves initial message if provided, then starts agent or sets up workspace in background.
   *
   * @param task - Task to start or setup
   * @param params - Task creation parameters
   * @deprecated Use createAndSetupWorkspaceInBackground() for non-blocking task creation
   */
  async handleTaskStartOrSetup(task: Task, params: CreateTaskRequest): Promise<void> {
    const shouldStart =
      (params.start ?? true) &&
      (Boolean(params.message) || (params.contexts && params.contexts.length > 0))

    if (shouldStart) {
      // Synchronous setup: wait for agent to start and first event
      await this.setupWorkspaceAndWaitForAgentStart(
        task.taskId,
        params.start,
        params.message,
        params.fileIds,
        params.contexts,
      )
    } else {
      // Background workspace setup only (no agent start)
      this.workspaceCoordinator.ensureWorkspace(task.taskId).catch((error) => {
        logger.error(
          { err: error as Error, taskId: task.taskId },
          'Background workspace setup failed',
        )
      })
    }
  }

  /**
   * Create and setup workspace in background, then auto-start task when ready.
   * This is the non-blocking version for Kubernetes deployments.
   *
   * The method:
   * 1. Provisions workspace (saves 'Pending' snapshot immediately for polling)
   * 2. Updates task with workspace path when ready
   * 3. Auto-starts task if there's a pending iteration with 'created' status
   *
   * Errors are saved in the workspace snapshot and can be retrieved via workspace-status API.
   *
   * @param task - Task to setup workspace for
   * @param params - Task creation parameters
   */
  createAndSetupWorkspaceInBackground(task: Task, params: CreateTaskRequest): void {
    const taskId = task.taskId
    const repositoryId = params.repositoryId

    // Run in background with session context preserved for access control
    runInBackgroundWithSession(async () => {
      try {
        logger.debug({ taskId, repositoryId }, 'Starting background workspace setup')

        // Provision workspace (saves 'Pending' snapshot first for frontend polling)
        const handle = await this.workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

        // Reload task to get latest state (may have been updated during provisioning)
        const updatedTask = await this.taskRepository.get(taskId)
        if (!updatedTask) {
          logger.warn({ taskId }, 'Task not found after workspace provisioning')
          return
        }

        // Update task with workspace path
        updatedTask.workspacePath = handle.workspacePath
        await this.taskRepository.save(updatedTask)

        logger.debug({ taskId, workspacePath: handle.workspacePath }, 'Workspace provisioned')

        // Auto-start task if there's a pending iteration
        const shouldStart =
          (params.start ?? true) &&
          (Boolean(params.message) || (params.contexts && params.contexts.length > 0))

        if (shouldStart && updatedTask.iterations.length > 0) {
          const lastIteration = updatedTask.iterations[updatedTask.iterations.length - 1]
          if (lastIteration?.status === 'created') {
            await this.startTask(taskId, params.fileIds, params.contexts)
            logger.info(
              { taskId, iterationNumber: lastIteration.iterationNumber },
              'Background workspace setup and task started.',
            )
          }
        } else {
          logger.info({ taskId }, 'Background workspace setup but task not started.')
        }
      } catch (error) {
        // Error is already saved in snapshot by ensureKubernetesWorkspace
        // Just log here for visibility
        logger.error(
          { err: error as Error, taskId, repositoryId },
          'Background workspace setup failed',
        )
      }
    })
  }

  /**
   * Setup workspace and wait for agent to start (used during task creation).
   *
   * @param taskId - Task ID
   * @param start - Whether to start the task
   * @param message - Initial message
   * @param fileIds - File attachments
   * @param contexts - Message contexts
   */
  async setupWorkspaceAndWaitForAgentStart(
    taskId: string,
    start?: boolean,
    message?: string,
    fileIds?: string[],
    contexts?: MessageContext[],
  ): Promise<void> {
    logger.debug({ taskId }, 'Setting up workspace and waiting for agent start')

    // Setup workspace (this is synchronous)
    try {
      await this.workspaceCoordinator.ensureWorkspace(taskId)
    } catch (error) {
      await this.handleWorkspaceSetupError(taskId, error)
      throw new Error(
        `Workspace setup failed: ${error instanceof Error ? error.message : String(error)}`,
      )
    }

    // Start task with waitForFirstEvent=true
    const shouldStart = (start ?? true) && (Boolean(message) || (contexts && contexts.length > 0))
    if (shouldStart) {
      await this.startTask(taskId, fileIds, contexts, true)
    }

    logger.debug(
      {
        taskId,
        started: shouldStart,
      },
      'Workspace setup and agent start completed',
    )
  }

  // Private helper methods

  /**
   * Execute agent in background and handle errors.
   *
   * Delegates to AgentController.runAgentInBackground() which:
   * 1. Spawns background task for agent execution
   * 2. Returns immediately
   *
   * Note: User message is already saved by sendMessage() before this is called,
   * so we pass userMessageSaved: true to skip duplicate save.
   */
  private async executeAgentInBackground(
    taskId: string,
    agentRequest: AgentRequest,
    iteration: TaskIteration,
    task: Task,
  ): Promise<void> {
    // Build run options for AgentController
    // Include userMessageSaved: true since message was already saved in sendMessage()
    const runOptions: AgentRunOptions = {
      iterationNumber: iteration.iterationNumber,
      userMessageSaved: true, // Message already saved in sendMessage() before ensureWorkspace()
      ...(agentRequest.resumeSessionId !== undefined && { resume: agentRequest.resumeSessionId }),
      ...(agentRequest.model !== undefined && { model: agentRequest.model }),
      ...(iteration.agentMode !== undefined && { agentMode: iteration.agentMode }),
      ...(agentRequest.fileIds !== undefined &&
        agentRequest.fileIds.length > 0 && { fileIds: agentRequest.fileIds }),
      ...(agentRequest.contexts !== undefined &&
        agentRequest.contexts.length > 0 && { contexts: agentRequest.contexts }),
      ...(iteration.background !== undefined && { background: iteration.background }),
    }

    // Delegate to AgentController which handles background execution
    await this.agentController.runAgentInBackground(
      taskId,
      agentRequest.agentType || 'claude-code',
      agentRequest.message,
      runOptions,
      iteration,
      task.repositoryId,
      task,
    )
  }

  /**
   * Wait for workspace to be ready by polling snapshots.
   */
  private async waitForWorkspaceReady(taskId: string): Promise<void> {
    const maxWaitTime = TaskExecutionCoordinator.WORKSPACE_READY_TIMEOUT
    const checkInterval = TaskExecutionCoordinator.WORKSPACE_CHECK_INTERVAL
    const startWait = Date.now()

    while (Date.now() - startWait < maxWaitTime) {
      const snapshot = await this.workspaceCoordinator.getSnapshot(taskId)
      if (snapshot) {
        // Unified availability check for both local and Kubernetes workspaces
        if (await this.workspaceCoordinator.isWorkspaceAvailable(taskId)) {
          return
        }
        // Check for errors in execution target
        if (snapshot.executionTarget.lastError) {
          throw new Error(
            `Cannot start task - workspace setup failed: ${snapshot.executionTarget.lastError}`,
          )
        }
      }
      await new Promise((resolve) => setTimeout(resolve, checkInterval))
    }

    throw new Error('Workspace ready timeout')
  }

  /**
   * Handle workspace setup errors by failing the iteration.
   */
  private async handleWorkspaceSetupError(taskId: string, error: unknown): Promise<void> {
    const task = await this.taskRepository.get(taskId)
    if (!task || task.iterations.length === 0) {
      logger.error({ taskId }, 'Workspace setup failed, skipping task start')
      return
    }

    const lastIteration = task.iterations[task.iterations.length - 1]
    if (!lastIteration || lastIteration.status !== 'created') {
      logger.error({ taskId }, 'Workspace setup failed, skipping task start')
      return
    }

    const errorMessage = error instanceof Error ? error.message : String(error)

    lastIteration.status = 'failed'
    lastIteration.result = `Workspace setup failed: ${errorMessage}`
    lastIteration.endTime = new Date()

    await this.taskRepository.save(task)

    await this.eventService.emitEvent({
      eventId: randomUUID(),
      taskId,
      type: 'taskIteration',
      subType: 'error',
      agentType: task.agentType,
      iterationId: lastIteration.iterationNumber,
      timestamp: new Date(),
      isError: true,
      payload: {
        status: 'error',
        previousStatus: 'created',
        iterationNumber: lastIteration.iterationNumber,
      },
    })

    logger.error({ taskId, error: errorMessage }, 'Workspace setup failed, skipping task start')
  }
}
