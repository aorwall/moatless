import { randomUUID } from 'node:crypto'
import type { CreateTaskRequest, Event, MessageContext, Task, TaskIteration } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { AgentRunOptions } from '../../agents/implementations/base-agent'
import type { AgentController } from '../../agents/services/agent-controller.service'
import { CommandService } from '../../agents/services/command.service'
import { AgentControllerSymbol } from '../../agents/symbols'
import { EventService } from '../../events/index'
import { RepositoryService } from '../../repositories/index'
import { FileGateway } from '../../workspace/services/file-gateway.service'
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
  private static readonly FIRST_EVENT_TIMEOUT = 30000 // 30 seconds

  constructor(
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(TaskIterationService) private readonly iterationService: TaskIterationService,
    @inject(WorkspaceCoordinator) private readonly workspaceCoordinator: WorkspaceCoordinator,
    @inject(FileGateway) private readonly fileGateway: FileGateway,
    @inject(AgentControllerSymbol) private readonly agentController: AgentController,
    @inject(EventService) private readonly eventService: EventService,
    @inject(RepositoryService) private readonly repositoryService: RepositoryService,
    @inject(CommandService) private readonly commandService: CommandService,
  ) {}

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
  ): Promise<void> {
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Mark last iteration as willNotResume if resume is false
    if (!resume && task.iterations.length > 0) {
      const lastIteration = task.iterations[task.iterations.length - 1]
      if (lastIteration && !lastIteration.willNotResume) {
        lastIteration.willNotResume = true
        await this.taskRepository.save(task)

        logger.info('Marked last iteration as will not resume due to resume=false', {
          taskId: task.taskId,
          iterationNumber: lastIteration.iterationNumber,
        })
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
    )

    // Check if command has willNotResume flag set
    if (commandName) {
      const command = await this.commandService.getCommand(commandName)
      if (command?.willNotResume) {
        iteration.willNotResume = true
        logger.info('Marked iteration as will not resume due to command flag', {
          taskId: task.taskId,
          commandName,
          iterationNumber: iteration.iterationNumber,
        })
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
    await this.executeAgentInBackground(taskId, agentRequest, iteration, task)
  }

  /**
   * Start a task by activating the last 'created' iteration.
   *
   * This method:
   * 1. Validates last iteration is in 'created' status
   * 2. Waits for workspace to be ready
   * 3. Updates iteration to 'active' and emits event
   * 4. Executes the agent
   * 5. Optionally waits for the first event (for synchronous creation)
   *
   * @param taskId - Task ID to start
   * @param fileIds - Optional file attachments
   * @param contexts - Optional message contexts
   * @param waitForFirstEvent - Whether to wait for first agent event
   */
  async startTask(
    taskId: string,
    fileIds?: string[],
    contexts?: MessageContext[],
    waitForFirstEvent = false,
  ): Promise<void> {
    logger.debug('Starting task', {
      taskId,
      hasFileIds: (fileIds?.length ?? 0) > 0,
      hasContexts: (contexts?.length ?? 0) > 0,
    })

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
      }

      // Get the agent event generator from AgentController (now handles message prep and enrichment)
      const eventGenerator = this.agentController.startAgent(
        taskId,
        'claude-code',
        lastIteration.initialMessage || '',
        runOptions,
        lastIteration,
        task.repositoryId,
      )

      // Wait for first event if requested (for synchronous task creation)
      if (waitForFirstEvent) {
        await this.waitForFirstAgentEvent(taskId, lastIteration, eventGenerator)
      }

      // Process remaining events in background
      await this.executeAgentEventsInBackground(taskId, lastIteration, eventGenerator, task)
    }

    logger.debug('Task started successfully', {
      taskId,
      iterationNumber: lastIteration.iterationNumber,
    })
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
      logger.error('Failed to create workspace', error as Error, {
        taskId: task.taskId,
        repositoryId,
      })
      throw error
    }
  }

  /**
   * Handle task start or background setup based on params.
   * Saves initial message if provided, then starts agent or sets up workspace in background.
   *
   * @param task - Task to start or setup
   * @param params - Task creation parameters
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
        logger.error('Background workspace setup failed', error as Error, {
          taskId: task.taskId,
        })
      })
    }
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
    logger.debug('Setting up workspace and waiting for agent start', { taskId })

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

    logger.debug('Workspace setup and agent start completed', {
      taskId,
      started: shouldStart,
    })
  }

  // Private helper methods

  /**
   * Execute agent in background and handle errors.
   * AgentController now handles message prep, system prompt, and event enrichment.
   */
  private executeAgentInBackground(
    taskId: string,
    agentRequest: AgentRequest,
    iteration: TaskIteration,
    task: Task,
  ): void {
    // biome-ignore lint/complexity/noVoid: Background task needs to run without awaiting
    void (async () => {
      // Build run options for AgentController
      const runOptions: AgentRunOptions = {
        iterationNumber: iteration.iterationNumber,
        ...(agentRequest.resumeSessionId !== undefined && { resume: agentRequest.resumeSessionId }),
        ...(agentRequest.model !== undefined && { model: agentRequest.model }),
        ...(iteration.agentMode !== undefined && { agentMode: iteration.agentMode }),
        ...(agentRequest.fileIds !== undefined &&
          agentRequest.fileIds.length > 0 && { fileIds: agentRequest.fileIds }),
        ...(agentRequest.contexts !== undefined &&
          agentRequest.contexts.length > 0 && { contexts: agentRequest.contexts }),
      }

      // Execute agent via AgentController (now handles message prep and enrichment)
      for await (const event of this.agentController.startAgent(
        taskId,
        agentRequest.agentType || 'claude-code',
        agentRequest.message,
        runOptions,
        iteration,
        task.repositoryId,
      )) {
        // Emit enriched event (AgentController already enriched it)
        await this.eventService.emitEvent(event)
      }

      // Save task with updated iteration
      await this.taskRepository.save(task)
    })().catch(async (error) => {
      logger.error('Background agent execution failed', error as Error, {
        taskId,
      })

      // Update status to error on agent failure
      try {
        // Only update iteration status, not task status
        iteration.status = 'failed'
        const errorMessage = error instanceof Error ? error.message : String(error)
        iteration.result = `Background execution failed: ${errorMessage || 'Unknown error'}. Model: ${agentRequest.model || 'default'}, Resuming: ${Boolean(agentRequest.resumeSessionId)}`
        iteration.endTime = new Date()

        await this.taskRepository.save(task)

        // Emit error event
        await this.eventService.emitEvent({
          eventId: randomUUID(),
          taskId,
          type: 'taskIteration',
          subType: 'error',
          agentType: task.agentType,
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
  }

  /**
   * Wait for first agent event (for synchronous task creation).
   * AgentController now handles session capture and enrichment.
   */
  private async waitForFirstAgentEvent(
    taskId: string,
    iteration: TaskIteration,
    eventGenerator: AsyncGenerator<Event>,
  ): Promise<void> {
    const firstEventPromise = eventGenerator.next()
    const timeoutPromise = new Promise<never>((_, reject) =>
      setTimeout(
        () => reject(new Error('Agent start timeout - first event not received within 30s')),
        TaskExecutionCoordinator.FIRST_EVENT_TIMEOUT,
      ),
    )

    try {
      const firstResult = await Promise.race([firstEventPromise, timeoutPromise])

      if (!firstResult.done && firstResult.value) {
        const event = firstResult.value

        // Emit enriched event (AgentController already enriched it)
        await this.eventService.emitEvent(event)

        logger.debug('First agent event received and emitted', {
          taskId,
          eventType: event.type,
          eventSubType: 'subType' in event ? event.subType : undefined,
        })
      } else if (firstResult.done) {
        throw new Error('Agent completed without emitting any events')
      }
    } catch (error) {
      logger.error('Failed to receive first agent event', error as Error, {
        taskId,
        iterationNumber: iteration.iterationNumber,
      })

      // Update iteration status to failed
      const updatedTask = await this.taskRepository.get(taskId)
      if (updatedTask) {
        const updatedIteration = updatedTask.iterations.find(
          (it) => it.iterationNumber === iteration.iterationNumber,
        )
        if (updatedIteration) {
          updatedIteration.status = 'failed'
          updatedIteration.result = `Agent failed to start: ${error instanceof Error ? error.message : String(error)}`
          updatedIteration.endTime = new Date()
          await this.taskRepository.save(updatedTask)

          // Emit error event
          await this.eventService.emitEvent({
            eventId: randomUUID(),
            taskId,
            type: 'taskIteration',
            subType: 'error',
            agentType: updatedTask.agentType,
            iterationId: iteration.iterationNumber,
            timestamp: new Date(),
            isError: true,
            payload: {
              status: 'error',
              previousStatus: 'active',
              iterationNumber: iteration.iterationNumber,
            },
          })
        }
      }

      throw error
    }
  }

  /**
   * Execute remaining agent events in background.
   * AgentController now handles session capture and enrichment.
   */
  private executeAgentEventsInBackground(
    taskId: string,
    iteration: TaskIteration,
    eventGenerator: AsyncGenerator<Event>,
    task: Task,
  ): void {
    // biome-ignore lint/complexity/noVoid: Background task needs to run without awaiting
    void (async () => {
      for await (const event of eventGenerator) {
        // Emit enriched event (AgentController already enriched it)
        await this.eventService.emitEvent(event)
      }

      // Save task with updated iteration
      await this.taskRepository.save(task)
    })().catch(async (error) => {
      logger.error('Failed to execute agent during task start', error as Error, {
        taskId,
        iterationNumber: iteration.iterationNumber,
      })

      // Update status to error on agent failure
      try {
        const updatedTask = await this.taskRepository.get(taskId)
        if (updatedTask) {
          const updatedIteration = updatedTask.iterations.find(
            (it) => it.iterationNumber === iteration.iterationNumber,
          )
          if (updatedIteration) {
            updatedIteration.status = 'failed'
            updatedIteration.result = `Agent failed to start: ${error instanceof Error ? error.message : String(error)}`
            updatedIteration.endTime = new Date()

            await this.taskRepository.save(updatedTask)

            // Emit error event
            await this.eventService.emitEvent({
              eventId: randomUUID(),
              taskId,
              type: 'taskIteration',
              subType: 'error',
              agentType: updatedTask.agentType,
              iterationId: iteration.iterationNumber,
              timestamp: new Date(),
              isError: true,
              payload: {
                status: 'error',
                previousStatus: iteration.status,
                iterationNumber: iteration.iterationNumber,
              },
            })
          }
        }
      } catch (saveError) {
        logger.error('Failed to update task status after agent error', saveError as Error)
      }
    })
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
        // Check if workspace path is valid
        const isValid = await this.fileGateway.isWorkspacePathValid(taskId)
        if (isValid) {
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
      logger.error('Workspace setup failed, skipping task start', { taskId })
      return
    }

    const lastIteration = task.iterations[task.iterations.length - 1]
    if (!lastIteration || lastIteration.status !== 'created') {
      logger.error('Workspace setup failed, skipping task start', { taskId })
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

    logger.error('Workspace setup failed, skipping task start', { taskId, error: errorMessage })
  }
}
