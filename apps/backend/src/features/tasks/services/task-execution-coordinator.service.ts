import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { Event, MessageContext, Task, TaskIteration } from '@moatless/types'
import { generateComponentList, TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { CommandService } from '../../../services/command.service'
import { UploadService } from '../../../services/upload.service'
import type { StorageBackend } from '../../../storage'
import { StorageSymbol } from '../../../storage/storage-backend'
import type { Logger } from '../../../utils/logger'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/index'
import { FigmaService } from '../../figma'
import { RepositoryService } from '../../repositories/index'
import { FileGateway, WorkspaceCoordinator } from '../../workspace/index'
import { AgentController } from '../../workspace/services/agent-controller.service'
import { TaskMetadataStorage } from '../storage/task-metadata.storage'
import { MessageService } from './message.service'
import { TaskIterationService } from './task-iteration.service'
import { TaskLifecycleService } from './task-lifecycle.service'

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
    @inject(TaskMetadataStorage) private readonly taskStorage: TaskMetadataStorage,
    @inject(TaskLifecycleService) private readonly lifecycleService: TaskLifecycleService,
    @inject(TaskIterationService) private readonly iterationService: TaskIterationService,
    @inject(MessageService) private readonly messageService: MessageService,
    @inject(WorkspaceCoordinator) private readonly workspaceCoordinator: WorkspaceCoordinator,
    @inject(FileGateway) private readonly fileGateway: FileGateway,
    @inject(AgentController) private readonly agentController: AgentController,
    @inject(EventService) private readonly eventService: EventService,
    @inject(RepositoryService) private readonly repositoryService: RepositoryService,
    @inject(CommandService) private readonly commandService: CommandService,
    @inject(UploadService) private readonly uploadService: UploadService,
    @inject(FigmaService) private readonly figmaService: FigmaService,
    @inject(StorageSymbol) private readonly storage: StorageBackend,
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
    const task = await this.lifecycleService.getTask(taskId)

    // Mark last iteration as willNotResume if resume is false
    if (!resume && task.iterations.length > 0) {
      const lastIteration = task.iterations[task.iterations.length - 1]
      if (lastIteration && !lastIteration.willNotResume) {
        lastIteration.willNotResume = true
        await this.taskStorage.save(task)

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
    await this.taskStorage.save(task)

    // Now start the iteration by updating its status to 'active' and starting execution
    iteration.status = 'active'

    // Save task with updated statuses
    await this.taskStorage.save(task)

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
    logger.info('Starting task', {
      taskId,
      hasFileIds: (fileIds?.length ?? 0) > 0,
      hasContexts: (contexts?.length ?? 0) > 0,
    })

    const task = await this.taskStorage.get(taskId)
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
    await this.taskStorage.save(task)

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
      const agentLogger = this.createAgentLogger(taskId, lastIteration.iterationNumber)

      // Fetch repository to get system prompt
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)

      // Prepare message with files and contexts
      let preparedMessage = await this.copyFilesToWorkspace(
        taskId,
        lastIteration.initialMessage || '',
        fileIds,
        agentLogger,
      )
      preparedMessage = await this.appendContextsToMessage(
        taskId,
        preparedMessage,
        contexts,
        agentLogger,
      )

      // Build system prompt
      const hasPreviewServer = await this.checkPreviewServerAvailability(taskId, agentLogger)
      const systemPromptParts = await this.generateSystemPromptParts(
        taskId,
        lastIteration,
        hasPreviewServer,
        repository?.systemPrompt,
      )

      // Build run options for AgentController
      const runOptions: {
        workspaceDir?: string
        iterationNumber?: number
        resume?: string
        model?: string
        agentMode?: 'edit' | 'plan'
        hasPreviewServer?: boolean
        appendSystemPrompt?: string
      } = {
        iterationNumber: lastIteration.iterationNumber,
        ...(lastIteration.model !== undefined && { model: lastIteration.model }),
        ...(lastIteration.agentMode !== undefined && { agentMode: lastIteration.agentMode }),
        hasPreviewServer,
        ...(systemPromptParts.length > 0 && { appendSystemPrompt: systemPromptParts.join('\n\n') }),
      }

      // Get the agent event generator from AgentController
      const eventGenerator = this.agentController.startAgent(
        taskId,
        'claude-code',
        preparedMessage,
        runOptions,
      )

      // Wait for first event if requested (for synchronous task creation)
      if (waitForFirstEvent) {
        await this.waitForFirstAgentEvent(taskId, lastIteration, eventGenerator, agentLogger)
      }

      // Process remaining events in background
      await this.executeAgentEventsInBackground(taskId, lastIteration, eventGenerator, agentLogger)
    }

    logger.info('Task started successfully', {
      taskId,
      iterationNumber: lastIteration.iterationNumber,
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
    logger.info('Setting up workspace and waiting for agent start', { taskId })

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

    logger.info('Workspace setup and agent start completed', {
      taskId,
      started: shouldStart,
    })
  }

  // Private helper methods

  /**
   * Execute agent in background and handle errors.
   */
  private executeAgentInBackground(
    taskId: string,
    agentRequest: AgentRequest,
    iteration: TaskIteration,
    task: Task,
  ): void {
    const agentLogger = this.createAgentLogger(taskId, iteration.iterationNumber)

    // biome-ignore lint/complexity/noVoid: Background task needs to run without awaiting
    // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex agent execution workflow with error handling
    void (async () => {
      // Prepare message with files and contexts
      let preparedMessage = await this.copyFilesToWorkspace(
        taskId,
        agentRequest.message,
        agentRequest.fileIds,
        agentLogger,
      )
      preparedMessage = await this.appendContextsToMessage(
        taskId,
        preparedMessage,
        agentRequest.contexts,
        agentLogger,
      )

      // Build system prompt
      const hasPreviewServer = await this.checkPreviewServerAvailability(taskId, agentLogger)
      const systemPromptParts = await this.generateSystemPromptParts(
        taskId,
        iteration,
        hasPreviewServer,
        agentRequest.systemPrompt,
      )

      // Build run options for AgentController
      const runOptions: {
        workspaceDir?: string
        iterationNumber?: number
        resume?: string
        model?: string
        agentMode?: 'edit' | 'plan'
        hasPreviewServer?: boolean
        appendSystemPrompt?: string
      } = {
        iterationNumber: iteration.iterationNumber,
        ...(agentRequest.resumeSessionId !== undefined && { resume: agentRequest.resumeSessionId }),
        ...(agentRequest.model !== undefined && { model: agentRequest.model }),
        ...(iteration.agentMode !== undefined && { agentMode: iteration.agentMode }),
        hasPreviewServer,
        ...(systemPromptParts.length > 0 && { appendSystemPrompt: systemPromptParts.join('\n\n') }),
      }

      // Execute agent via AgentController
      for await (const event of this.agentController.startAgent(
        taskId,
        agentRequest.agentType || 'claude-code',
        preparedMessage,
        runOptions,
      )) {
        // Capture session ID from agent events
        if (event.type === 'agent' && !iteration.agentSessionId) {
          const sessionId =
            ('agentSessionId' in event.payload && event.payload.agentSessionId) || undefined
          if (sessionId && typeof sessionId === 'string') {
            iteration.agentSessionId = sessionId
            agentLogger.debug('Captured agent session ID', { sessionId })
          }
        }

        // Save message events
        if (event.type === 'agent' && event.subType === 'message') {
          this.messageService
            .saveAgentMessage(taskId, {
              ...event,
              payload: {
                ...event.payload,
                agentSessionId: iteration.agentSessionId,
              },
            })
            .catch((error) => {
              agentLogger.debug('Message persistence completed with warning', {
                eventId: event.eventId,
                error: error instanceof Error ? error.message : String(error),
              })
            })
        }

        // Handle completion events
        if (event.type === 'agent' && event.subType === 'completed') {
          const resultPayload = event.payload
          agentLogger.info('Agent execution completed', {
            costUsd: resultPayload.total_cost_usd,
            hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
            isError: event.isError,
            subtype: resultPayload.subtype,
          })

          iteration.costUsd = resultPayload.total_cost_usd ?? 0
          iteration.endTime = new Date()
        }

        // Enrich event with session ID and emit
        await this.eventService.emitEvent({
          ...event,
          payload: {
            ...event.payload,
            agentSessionId: iteration.agentSessionId,
          },
        })
      }
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

        await this.taskStorage.save(task)

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
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex event handling and timeout logic necessary
  private async waitForFirstAgentEvent(
    taskId: string,
    iteration: TaskIteration,
    eventGenerator: AsyncGenerator<Event>,
    agentLogger: Logger,
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

        // Capture session ID from agent events
        if (event.type === 'agent' && !iteration.agentSessionId) {
          const sessionId =
            ('agentSessionId' in event.payload && event.payload.agentSessionId) || undefined
          if (sessionId && typeof sessionId === 'string') {
            iteration.agentSessionId = sessionId
            agentLogger.debug('Captured agent session ID', { sessionId })
          }
        }

        // Save message events
        if (event.type === 'agent' && event.subType === 'message') {
          this.messageService
            .saveAgentMessage(taskId, {
              ...event,
              payload: {
                ...event.payload,
                agentSessionId: iteration.agentSessionId,
              },
            })
            .catch((error) => {
              agentLogger.debug('Message persistence completed with warning', {
                eventId: event.eventId,
                error: error instanceof Error ? error.message : String(error),
              })
            })
        }

        // Handle completion events
        if (event.type === 'agent' && event.subType === 'completed') {
          const resultPayload = event.payload
          agentLogger.info('Agent execution completed', {
            costUsd: resultPayload.total_cost_usd,
            hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
            isError: event.isError,
            subtype: resultPayload.subtype,
          })

          iteration.costUsd = resultPayload.total_cost_usd ?? 0
          iteration.endTime = new Date()
        }

        // Enrich event with session ID and emit
        await this.eventService.emitEvent({
          ...event,
          payload: {
            ...event.payload,
            agentSessionId: iteration.agentSessionId,
          },
        })

        logger.info('First agent event received and emitted', {
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
      const updatedTask = await this.taskStorage.get(taskId)
      if (updatedTask) {
        const updatedIteration = updatedTask.iterations.find(
          (it) => it.iterationNumber === iteration.iterationNumber,
        )
        if (updatedIteration) {
          updatedIteration.status = 'failed'
          updatedIteration.result = `Agent failed to start: ${error instanceof Error ? error.message : String(error)}`
          updatedIteration.endTime = new Date()
          await this.taskStorage.save(updatedTask)

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
   */
  private executeAgentEventsInBackground(
    taskId: string,
    iteration: TaskIteration,
    eventGenerator: AsyncGenerator<Event>,
    agentLogger: Logger,
  ): void {
    // biome-ignore lint/complexity/noVoid: Background task needs to run without awaiting
    // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex event processing loop with error handling
    void (async () => {
      for await (const event of eventGenerator) {
        // Capture session ID from agent events
        if (event.type === 'agent' && !iteration.agentSessionId) {
          const sessionId =
            ('agentSessionId' in event.payload && event.payload.agentSessionId) || undefined
          if (sessionId && typeof sessionId === 'string') {
            iteration.agentSessionId = sessionId
            agentLogger.debug('Captured agent session ID', { sessionId })
          }
        }

        // Save message events
        if (event.type === 'agent' && event.subType === 'message') {
          this.messageService
            .saveAgentMessage(taskId, {
              ...event,
              payload: {
                ...event.payload,
                agentSessionId: iteration.agentSessionId,
              },
            })
            .catch((error) => {
              agentLogger.debug('Message persistence completed with warning', {
                eventId: event.eventId,
                error: error instanceof Error ? error.message : String(error),
              })
            })
        }

        // Handle completion events
        if (event.type === 'agent' && event.subType === 'completed') {
          const resultPayload = event.payload
          agentLogger.info('Agent execution completed', {
            costUsd: resultPayload.total_cost_usd,
            hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
            isError: event.isError,
            subtype: resultPayload.subtype,
          })

          iteration.costUsd = resultPayload.total_cost_usd ?? 0
          iteration.endTime = new Date()
        }

        // Enrich event with session ID and emit
        await this.eventService.emitEvent({
          ...event,
          payload: {
            ...event.payload,
            agentSessionId: iteration.agentSessionId,
          },
        })
      }
    })().catch(async (error) => {
      logger.error('Failed to execute agent during task start', error as Error, {
        taskId,
        iterationNumber: iteration.iterationNumber,
      })

      // Update status to error on agent failure
      try {
        const updatedTask = await this.taskStorage.get(taskId)
        if (updatedTask) {
          const updatedIteration = updatedTask.iterations.find(
            (it) => it.iterationNumber === iteration.iterationNumber,
          )
          if (updatedIteration) {
            updatedIteration.status = 'failed'
            updatedIteration.result = `Agent failed to start: ${error instanceof Error ? error.message : String(error)}`
            updatedIteration.endTime = new Date()

            await this.taskStorage.save(updatedTask)

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
    const task = await this.taskStorage.get(taskId)
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

    await this.taskStorage.save(task)

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

  /**
   * Copy uploaded files to workspace so agent can access them
   */
  private async copyFilesToWorkspace(
    taskId: string,
    message: string,
    fileIds: string[] | undefined,
    agentLogger: Logger,
  ): Promise<string> {
    if (!fileIds || fileIds.length === 0) {
      return message
    }

    // Get workspace path from coordinator
    const snapshot = await this.workspaceCoordinator.getSnapshot(taskId)
    if (!snapshot) {
      throw new Error('Cannot copy files - workspace not found')
    }

    const workspacePath = snapshot.executionTarget.workspacePath
    const uploadsDir = path.join(workspacePath, 'uploads')

    // Create uploads directory in workspace
    await fs.mkdir(uploadsDir, { recursive: true })

    const copiedFiles: string[] = []

    for (const fileId of fileIds) {
      try {
        // Get file from upload service
        const { buffer, metadata } = await this.uploadService.getFile(fileId)

        // Copy to workspace uploads directory
        const targetPath = path.join(uploadsDir, metadata.filename)
        await fs.writeFile(targetPath, buffer)

        copiedFiles.push(targetPath)
        agentLogger.info('File copied to workspace', {
          fileId,
          filename: metadata.filename,
          targetPath,
        })
      } catch (error) {
        agentLogger.warn('Failed to copy file to workspace', {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    // Append file paths to message for agent context
    if (copiedFiles.length > 0) {
      const finalMessage = message
        ? `${message}\n\nUploaded files:\n${copiedFiles.join('\n')}`
        : `Uploaded files:\n${copiedFiles.join('\n')}`
      agentLogger.info('Files copied to workspace and message prepared', {
        fileCount: copiedFiles.length,
        messageLength: finalMessage.length,
      })
      return finalMessage
    }

    return message
  }

  /**
   * Append context items to message for agent reference
   */
  private async appendContextsToMessage(
    taskId: string,
    message: string,
    contexts: MessageContext[] | undefined,
    agentLogger: Logger,
  ): Promise<string> {
    if (!contexts || contexts.length === 0) {
      return message
    }

    const contextParts: string[] = []

    for (const context of contexts) {
      if (context.type === 'element') {
        const componentList = generateComponentList(context.data)
        contextParts.push(componentList)
      } else if (context.type === 'figma') {
        // Handle Figma context
        const figmaPart = await this.processFigmaContext(taskId, context.data, agentLogger)
        if (figmaPart) {
          contextParts.push(figmaPart)
        }
      }
      // Future context types can be added here
    }

    if (contextParts.length > 0) {
      if (message) {
        return `${message}\n\nContext:\n${contextParts.join('\n\n')}`
      }
      return `Context:\n${contextParts.join('\n\n')}`
    }

    return message
  }

  /**
   * Process Figma context by copying image to workspace and returning formatted context
   */
  private async processFigmaContext(
    taskId: string,
    figmaData: { importId: string; fileName: string; nodeName: string },
    agentLogger: Logger,
  ): Promise<string | null> {
    try {
      // Get workspace path from coordinator
      const snapshot = await this.workspaceCoordinator.getSnapshot(taskId)
      if (!snapshot) {
        throw new Error('Cannot process Figma context - workspace not found')
      }

      const workspacePath = snapshot.executionTarget.workspacePath

      // Use FigmaService to get context for workspace
      const contextText = await this.figmaService.getFigmaContextForWorkspace(
        figmaData.importId,
        workspacePath,
      )

      agentLogger.debug('Figma context processed successfully', {
        importId: figmaData.importId,
      })

      return contextText
    } catch (error) {
      agentLogger.warn('Failed to process Figma context', {
        importId: figmaData.importId,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }

  private async checkPreviewServerAvailability(
    taskId: string,
    agentLogger: Logger,
  ): Promise<boolean> {
    try {
      agentLogger.debug('Checking preview server availability', { taskId })

      const task = await this.storage.getTask(taskId)
      if (task) {
        const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
        const hasPreviewServer = (repository?.previewConfig?.serverStartCommands?.length ?? 0) > 0

        agentLogger.info('Preview server availability check result', {
          taskId,
          repositoryId: task.repositoryId,
          hasPreviewServer,
          serverStartCommandsCount: repository?.previewConfig?.serverStartCommands?.length ?? 0,
        })

        return hasPreviewServer
      }

      agentLogger.info('Preview server availability check - no task found', { taskId })
      return false
    } catch (error) {
      agentLogger.warn('Failed to check preview server availability', {
        error: error instanceof Error ? error.message : String(error),
      })
      return false
    }
  }

  private async generateSystemPromptParts(
    taskId: string,
    iteration: TaskIteration,
    hasPreviewServer: boolean,
    systemPrompt: string | undefined,
  ): Promise<string[]> {
    const parts = [
      systemPrompt ?? null,
      await this.getTaskMetadataPrompt(taskId, iteration),
      ...this.getPreviewServerPrompt(hasPreviewServer),
      ...(await this.getSetupCommandsPrompt(taskId)),
      ...this.getRepositoryManagementPrompt(),
    ]
    return parts.filter((p): p is string => p !== null)
  }

  private async getTaskMetadataPrompt(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<string | null> {
    if (iteration.iterationNumber !== 1) {
      return null
    }

    try {
      const task = await this.storage.getTask(taskId)
      if (!task?.name) {
        return "IMPORTANT: Before starting work on this task, please use the 'mcp__moatless-tools__updateTaskMetadata' tool to set a clear task name and description (max 2 sentences or 25 words) based on what you need to accomplish."
      }
      return null
    } catch {
      return null
    }
  }

  private getPreviewServerPrompt(hasPreviewServer: boolean): string[] {
    if (!hasPreviewServer) {
      return []
    }

    return [
      "**Interface Layout**: On the left hand side of the interface, there's a chat window where users chat with you. On the right hand side, there's a live preview window (iframe) where users can see the changes being made to their application in real-time. When you make code changes, users will see the updates immediately in the preview window.",
      '**CRITICAL - Preview Server Management**: A preview server is running for this task. You MUST use the provided MCP tools to manage the server - DO NOT use bash commands like `npm start`, `yarn dev`, `bun dev`, or any other manual server commands. Use these MCP tools instead:',
      '- `mcp__moatless-tools__startOrRestartPreviewServer` - Start or restart the preview server',
      '- `mcp__moatless-tools__getPreviewServerLogs` - View server logs',
      'These tools ensure proper integration with the preview window. Manual server commands will conflict with the managed preview server.',
    ]
  }

  private async getSetupCommandsPrompt(taskId: string): Promise<string[]> {
    try {
      const task = await this.storage.getTask(taskId)
      if (!task) {
        return []
      }

      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      const setupCommands = repository?.codeExecution?.setupCommands

      if (!setupCommands || setupCommands.length === 0) {
        return []
      }

      return [
        '**Repository Setup Commands**: The following setup commands are configured for this repository:',
        ...setupCommands.map((cmd, index) => `  ${index + 1}. \`${cmd}\``),
        '',
        'You should run these commands when needed for the repository to work properly. You can use the `mcp__moatless-tools__install` tool to run all setup commands at once, or run them individually using bash commands.',
      ]
    } catch {
      return []
    }
  }

  private getRepositoryManagementPrompt(): string[] {
    return [
      '**Repository Configuration**: You have access to repository management tools:',
      '- `mcp__moatless-tools__install` - Install the repository by running setup commands (e.g., npm install, pip install)',
      '- `mcp__moatless-tools__updateRepositoryConfig` - Update repository configuration (setup commands, environment variables, server start commands)',
      '- `mcp__moatless-tools__getRepositoryConfig` - Get current repository configuration',
      'Use these tools to manage repository installation and configuration instead of manual commands when appropriate.',
    ]
  }

  private createAgentLogger(taskId: string, iterationNumber: number) {
    return logger.child({
      taskId,
      iterationNumber,
      component: 'TaskExecutionCoordinator',
    })
  }
}
