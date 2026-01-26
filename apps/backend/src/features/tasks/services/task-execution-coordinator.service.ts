import { randomUUID } from 'node:crypto'
import type { MessageContext, TaskIteration } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { AgentRequestService } from '../../agents/services/agent-request.service'
import { AgentRequestBuilder } from '../../agents/services/agent-request-builder.service'
import { CommandService } from '../../agents/services/command.service'
import { MessageService } from '../../agents/services/message.service'
import { runInBackgroundWithSession } from '../../auth'
import { EventService } from '../../events/index'
import { agentExecutionTotal } from '../../metrics'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../db/task.repository'
import { TaskIterationService } from './task-iteration.service'

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

  constructor(
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(TaskIterationService) private readonly iterationService: TaskIterationService,
    @inject(WorkspaceRuntimeManager) private readonly workspaceCoordinator: WorkspaceRuntimeManager,
    @inject(AgentRequestService) private readonly agentRequestService: AgentRequestService,
    @inject(EventService) private readonly eventService: EventService,
    @inject(CommandService) private readonly commandService: CommandService,
    @inject(MessageService) private readonly messageService: MessageService,
    @inject(AgentRequestBuilder) private readonly agentRequestBuilder: AgentRequestBuilder,
  ) {}

  /**
   * Ensure workspace is provisioning in the background (non-blocking).
   * This is called after publishing to queue to ensure workspace starts setting up.
   *
   * The method:
   * 1. Provisions workspace (saves 'Pending' snapshot immediately for polling)
   * 2. Updates task with workspace path when ready
   *
   * Agent execution is decoupled - the queue consumer will execute the agent
   * once the workspace is ready.
   *
   * Errors are saved in the workspace snapshot and can be retrieved via workspace-status API.
   *
   * @param taskId - Task ID
   * @param repositoryId - Repository ID for workspace setup
   */
  ensureWorkspaceInBackground(taskId: string, repositoryId: string): void {
    // Run in background with session context preserved for access control
    runInBackgroundWithSession(async () => {
      try {
        logger.debug({ taskId, repositoryId }, 'Starting background workspace setup')

        // Provision workspace (saves 'Pending' snapshot first for frontend polling)
        const handle = await this.workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

        logger.info({ taskId, workspacePath: handle.workspacePath }, 'Workspace provisioned')
        // Task execution is handled by the queue consumer when it receives the request
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

    taskLogger.info(
      { resumeSessionId, hasMessage: !!message },
      'Publishing resumed iteration to queue',
    )

    // Build and publish request to queue
    const snapshot = await this.workspaceCoordinator.getSnapshot(taskId)
    const agentRequest = await this.agentRequestBuilder.buildRequest(
      taskId,
      message,
      iteration,
      task.repositoryId,
      {
        resumeSessionId,
        externalPreviewUrl: snapshot?.preview?.proxyUrl,
      },
    )

    await this.agentRequestService.publish(taskId, agentRequest)

    // Ensure workspace is ready (non-blocking)
    this.ensureWorkspaceInBackground(taskId, task.repositoryId)

    agentExecutionTotal.labels({ status: 'queued', agent_type: 'claude-code' }).inc()
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
   * Send a message to a task, creating a new iteration and publishing to queue.
   *
   * Message Persistence Flow:
   * 1. Message is saved to MessageService FIRST (agentSessionId=null)
   * 2. This ensures message is immediately available via GET /messages
   * 3. Agent receives message when it starts (from queue)
   * 4. When agent emits first user/system event, message is marked as acknowledged
   *
   * This method:
   * 1. Saves user message to MessageService (agentSessionId=null)
   * 2. Loads the task
   * 3. Marks previous iteration as will-not-resume if needed
   * 4. Creates a new iteration
   * 5. Determines resume session ID
   * 6. Saves the task and emits events
   * 7. Builds AgentRunRequest and publishes to queue
   * 8. Ensures workspace provisioning in background
   *
   * @param taskId - Task ID
   * @param message - User message
   * @param resume - Whether to resume from previous iteration
   * @param model - Optional model override
   * @param fileIds - Optional file attachments
   * @param commandName - Optional command name
   * @param agentMode - Agent mode (edit/plan)
   * @param contexts - Optional message contexts
   * @param background - Optional background mode flag
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
    const taskLogger = logger.child({ taskId, handler: 'sendMessage' })

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
    taskLogger.info(
      { iterationNumber, hasContexts: (contexts?.length ?? 0) > 0 },
      'User message saved before queue publish',
    )

    // Determine effective background flag (per-message override > task-level)
    const effectiveBackground = background ?? task.background

    // Mark last iteration as willNotResume if resume is false
    if (!resume && task.iterations.length > 0) {
      const lastIteration = task.iterations[task.iterations.length - 1]
      if (lastIteration && !lastIteration.willNotResume) {
        lastIteration.willNotResume = true
        await this.taskRepository.save(task)

        taskLogger.info(
          { iterationNumber: lastIteration.iterationNumber },
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
        taskLogger.info(
          { commandName, iterationNumber: iteration.iterationNumber },
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

    // Now start the iteration by updating its status to 'active'
    iteration.status = 'active'

    // Save task with updated status
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

    // Build AgentRunRequest and publish to queue
    const snapshot = await this.workspaceCoordinator.getSnapshot(taskId)
    const agentRequest = await this.agentRequestBuilder.buildRequest(
      taskId,
      finalMessage,
      iteration,
      task.repositoryId,
      {
        resumeSessionId,
        fileIds,
        contexts,
        externalPreviewUrl: snapshot?.preview?.proxyUrl,
      },
    )

    await this.agentRequestService.publish(taskId, agentRequest)

    taskLogger.info(
      { iterationNumber: iteration.iterationNumber, hasResume: !!resumeSessionId },
      'Agent request published to queue',
    )

    // Ensure workspace is provisioning in background (non-blocking)
    this.ensureWorkspaceInBackground(taskId, task.repositoryId)

    agentExecutionTotal.labels({ status: 'queued', agent_type: 'claude-code' }).inc()
  }

  /**
   * Start a task by activating the last 'created' iteration.
   *
   * This method:
   * 1. Validates last iteration is in 'created' status
   * 2. Waits for workspace to be ready
   * 3. Updates iteration to 'active' and emits event
   * 4. Publishes agent request to queue
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
    const taskLogger = logger.child({ taskId, handler: 'startTask' })

    taskLogger.info(
      {
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

    // Publish agent request to queue
    if (lastIteration.initialMessage || lastIteration.commandName) {
      const snapshot = await this.workspaceCoordinator.getSnapshot(taskId)
      const agentRequest = await this.agentRequestBuilder.buildRequest(
        taskId,
        lastIteration.initialMessage || '',
        lastIteration,
        task.repositoryId,
        {
          fileIds,
          contexts,
          externalPreviewUrl: snapshot?.preview?.proxyUrl,
        },
      )

      await this.agentRequestService.publish(taskId, agentRequest)

      taskLogger.info(
        { iterationNumber: lastIteration.iterationNumber },
        'Agent request published to queue for task start',
      )

      agentExecutionTotal.labels({ status: 'queued', agent_type: 'claude-code' }).inc()
    }

    taskLogger.debug(
      { iterationNumber: lastIteration.iterationNumber },
      'Task started successfully',
    )
  }

  // ========================================
  // Private helper methods
  // ========================================

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
}
