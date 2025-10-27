import { randomUUID } from 'node:crypto'
import type {
  Command,
  CreateTaskRequest,
  CreateVibeTaskRequest,
  MessageContext,
  Task,
  TaskIteration,
} from '@moatless/types'
import { ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { StorageBackend } from '../../../storage/storage-backend'
import { StorageSymbol } from '../../../storage/storage-backend'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { RepositoryService } from '../../repositories/services/repository.service'
import { AgentController } from '../../workspace/services/agent-controller.service'
import { PreviewManager } from '../../workspace/services/preview-manager.service'
import { WorkspaceCoordinator } from '../../workspace/services/workspace-coordinator.service'
import { MessageService } from './message.service'
import { TaskCommandGateway } from './task-command-gateway.service'
import { TaskExecutionCoordinator } from './task-execution-coordinator.service'
import { TaskLifecycleService } from './task-lifecycle.service'

/**
 * TaskFacade
 *
 * Unified API for all task-related operations.
 * Orchestrates complex workflows and delegates to specialized services.
 *
 * @responsibilities
 * - Task creation orchestration (workspace setup, event emission)
 * - Task deletion orchestration (cleanup, storage deletion)
 * - Delegation to specialized services
 * - Public API for routes
 *
 * @dependencies
 * - TaskLifecycleService - Task CRUD operations
 * - TaskExecutionCoordinator - Agent execution
 * - TaskCommandGateway - Command filtering
 * - MessageService - Message operations
 * - WorkspaceCoordinator - Workspace management
 * - RepositoryService - Repository operations
 * - EventService - Event emission
 * - AgentController - Agent lifecycle
 * - PreviewManager - Preview server lifecycle
 * - Storage - Direct storage access (legacy compatibility)
 */
@injectable()
export class TaskService {
  constructor(
    @inject(TaskLifecycleService) private lifecycleService: TaskLifecycleService,
    @inject(TaskExecutionCoordinator) private executionCoordinator: TaskExecutionCoordinator,
    @inject(TaskCommandGateway) private commandGateway: TaskCommandGateway,
    @inject(MessageService) private messageService: MessageService,
    @inject(WorkspaceCoordinator) private workspaceCoordinator: WorkspaceCoordinator,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(EventService) private eventService: EventService,
    @inject(AgentController) private agentController: AgentController,
    @inject(PreviewManager) private previewManager: PreviewManager,
    @inject(StorageSymbol) private storage: StorageBackend,
  ) {}

  // ========================================
  // Task Lifecycle - Delegation Methods
  // ========================================

  /**
   * Get task by ID with enriched data
   * Delegates to TaskLifecycleService
   */
  async getTask(taskId: string): Promise<Task> {
    return await this.lifecycleService.getTask(taskId)
  }

  /**
   * List tasks with pagination and filtering
   * Delegates to TaskLifecycleService
   */
  async listTasks(
    limit = 50,
    offset = 0,
    status?: string,
  ): Promise<{ tasks: Task[]; total: number }> {
    return await this.lifecycleService.listTasks(limit, offset, status)
  }

  /**
   * Update task metadata (name, description)
   * Delegates to TaskLifecycleService
   */
  async updateTaskMetadata(
    taskId: string,
    updates: { name?: string; description?: string },
  ): Promise<Task> {
    await this.lifecycleService.updateTaskMetadata(taskId, updates)
    return this.lifecycleService.getTask(taskId)
  }

  /**
   * Update task with partial updates
   * Direct storage operation for legacy compatibility
   */
  async updateTask(taskId: string, updates: Partial<Task>): Promise<void> {
    const task = await this.getTask(taskId)
    const updatedTask = { ...task, ...updates }
    await this.storage.save(`tasks/${taskId}`, updatedTask)
  }

  // ========================================
  // Task Execution - Delegation Methods
  // ========================================

  /**
   * Start a task by activating the 'created' iteration
   * Delegates to TaskExecutionCoordinator
   */
  async startTask(
    taskId: string,
    fileIds?: string[],
    contexts?: MessageContext[],
    waitForFirstEvent = false,
  ): Promise<void> {
    return await this.executionCoordinator.startTask(taskId, fileIds, contexts, waitForFirstEvent)
  }

  /**
   * Send a message to a task
   * Delegates to TaskExecutionCoordinator
   */
  async sendMessage(
    taskId: string,
    message: string,
    resume?: boolean,
    model?: string,
    fileIds?: string[],
    commandName?: string,
    agentMode?: 'edit' | 'plan',
    contexts?: MessageContext[],
  ): Promise<void> {
    return await this.executionCoordinator.sendMessage(
      taskId,
      message,
      resume,
      model,
      fileIds,
      commandName,
      agentMode,
      contexts,
    )
  }

  /**
   * Send a command to a task
   * Delegates to TaskExecutionCoordinator
   */
  async sendCommand(
    taskId: string,
    commandName: string,
    resume?: boolean,
    model?: string,
    fileIds?: string[],
  ): Promise<void> {
    return await this.executionCoordinator.sendCommand(taskId, commandName, resume, model, fileIds)
  }

  // ========================================
  // Commands - Delegation Methods
  // ========================================

  /**
   * Get available commands for a task
   * Delegates to TaskCommandGateway
   */
  async getAvailableCommands(taskId: string): Promise<Command[]> {
    return await this.commandGateway.getAvailableCommands(taskId)
  }

  // ========================================
  // Messages - Delegation Methods
  // ========================================

  /**
   * Get task messages
   * Delegates to MessageService
   */
  async getTaskMessages(taskId: string) {
    return await this.messageService.getTaskMessages(taskId)
  }

  // ========================================
  // Task Creation - Orchestration
  // ========================================

  /**
   * Create a new task with complete orchestration
   *
   * Workflow:
   * 1. Validate name format
   * 2. Initialize task metadata
   * 3. Create initial iteration (if message provided)
   * 4. Persist task and emit created event
   * 5. Create workspace
   * 6. Handle start or background setup
   */
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
        params.agentMode,
        params.contexts,
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
   * Create a vibe task from template
   *
   * Workflow:
   * 1. Generate task ID
   * 2. Auto-generate repository name
   * 3. Create repository from template
   * 4. Create task with new repository
   */
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
    const createTaskParams: CreateTaskRequest = {
      agentType: params.agentType,
      repositoryId: repository.id,
      message: params.message,
      start: params.start,
      model: params.model,
      fileIds: params.fileIds,
      contexts: params.contexts,
      templateId: params.templateId,
      name: params.name,
      description: params.description,
    }

    const task = await this.createTask(createTaskParams)
    return task.taskId
  }

  // ========================================
  // Task Deletion - Orchestration
  // ========================================

  /**
   * Delete a task completely
   *
   * Workflow:
   * 1. Verify task exists
   * 2. Stop preview server
   * 3. Stop agent
   * 4. Delete workspace snapshot
   * 5. Delete task storage
   * 6. Delete event storage
   */
  async deleteTask(taskId: string): Promise<void> {
    await this.getTask(taskId)

    // Clean up workspace (stops agents, servers, containers, etc.)
    await this.previewManager.cleanup()
    await this.agentController.cleanup()
    await this.workspaceCoordinator.deleteSnapshot(taskId)

    // Delete task data
    await this.storage.delete(`tasks/${taskId}`)
    await this.storage.delete(`events/${taskId}`)
  }

  /**
   * Close a task (stop agents/servers, mark as closed)
   *
   * Workflow:
   * 1. Get task
   * 2. Stop agent
   * 3. Stop preview server
   * 4. Delete workspace snapshot
   * 5. Update task status to closed
   * 6. Emit closed event
   */
  async closeTask(taskId: string): Promise<void> {
    const task = await this.getTask(taskId)

    logger.debug('Closing task', { taskId })

    // Delete workspace snapshot
    await this.workspaceCoordinator.deleteWorkspace(taskId)

    // Update task status to closed
    const previousStatus = task.status
    task.status = 'closed'
    await this.storage.save(`tasks/${taskId}`, task)

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

  // ========================================
  // Private Helper Methods
  // ========================================

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
   * Create a task iteration object
   */
  private createTaskIteration(
    task: Task,
    message?: string,
    commandName?: string,
    model?: string,
    status: 'created' | 'active' = 'created',
    agentMode?: 'edit' | 'plan',
    _contexts?: MessageContext[], // contexts are handled by MessageService
  ): TaskIteration {
    const iterationNumber = task.iterations.length + 1
    return {
      iterationNumber,
      startTime: new Date(),
      status,
      messageCount: 0,
      costUsd: 0,
      initialMessage: message,
      commandName,
      model,
      agentMode,
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
      await this.storage.save(`tasks/${task.taskId}`, task)

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
      const handle = await this.workspaceCoordinator.ensureWorkspace(
        task.taskId,
        params.repositoryId,
      )
      task.workspacePath = handle.workspacePath

      // Save task with workspace path
      await this.storage.save(`tasks/${task.taskId}`, task)
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
   */
  private async handleTaskStartOrSetup(task: Task, params: CreateTaskRequest): Promise<void> {
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
          undefined,
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
   * Setup workspace and wait for agent to start
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

  /**
   * Handle workspace setup error by failing the iteration
   */
  private async handleWorkspaceSetupError(taskId: string, error: unknown): Promise<void> {
    const task = await this.storage.get<Task>(`tasks/${taskId}`)
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

    await this.storage.save(`tasks/${taskId}`, task)
    await this.emitIterationErrorEvent(taskId, task.agentType, lastIteration, 'created')

    logger.error('Workspace setup failed, skipping task start', { taskId, error: errorMessage })
  }

  /**
   * Emit iteration error event
   */
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
}
