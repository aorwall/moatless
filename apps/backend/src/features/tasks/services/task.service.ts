import { randomUUID } from 'node:crypto'
import type {
  Command,
  CreateTaskRequest,
  CreateVibeTaskRequest,
  MessageContext,
  Task,
  TaskIteration,
} from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { StorageBackend } from '../../../storage/storage-backend'
import { StorageSymbol } from '../../../storage/storage-backend'
import { logger } from '../../../utils/logger'
import type { AgentController } from '../../agents/services/agent-controller.service'
import { AgentControllerSymbol } from '../../agents/symbols'
import { EventService } from '../../events/services/event.service'
import { PreviewOrchestratorService } from '../../preview/services/preview-orchestrator.service'
import { RepositoryService } from '../../repositories/services/repository.service'
import { WorkspaceCoordinator } from '../../workspace/services/workspace-coordinator.service'
import { TaskRepository } from '../db/task.repository'
import { deriveTaskStatus } from '../utils/task-status.utils'
import { TaskCommandGateway } from './task-command-gateway.service'
import { TaskExecutionCoordinator } from './task-execution-coordinator.service'

/**
 * TaskService
 *
 * Unified API for all task-related operations.
 * Handles CRUD operations and orchestrates complex workflows.
 *
 * @responsibilities
 * - Task CRUD operations (get, list, update with status derivation and enrichment)
 * - Task creation orchestration (workspace setup, event emission)
 * - Task deletion orchestration (cleanup, storage deletion)
 * - Delegation to specialized services
 * - Public API for routes
 *
 * @dependencies
 * - TaskRepository - Task persistence
 * - TaskExecutionCoordinator - Agent execution and workspace management
 * - TaskCommandGateway - Command filtering
 * - WorkspaceCoordinator - Workspace management
 * - RepositoryService - Repository operations
 * - EventService - Event emission
 * - AgentController - Agent lifecycle
 * - PreviewOrchestratorService - Preview server lifecycle
 * - Storage - Direct storage access (legacy compatibility)
 */
@injectable()
export class TaskService {
  constructor(
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(TaskExecutionCoordinator) private executionCoordinator: TaskExecutionCoordinator,
    @inject(TaskCommandGateway) private commandGateway: TaskCommandGateway,
    @inject(WorkspaceCoordinator) private workspaceCoordinator: WorkspaceCoordinator,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(EventService) private eventService: EventService,
    @inject(AgentControllerSymbol) private agentController: AgentController,
    @inject(PreviewOrchestratorService) private previewOrchestrator: PreviewOrchestratorService,
    @inject(StorageSymbol) private storage: StorageBackend,
  ) {}

  // ========================================
  // Task CRUD Operations
  // ========================================

  /**
   * Get a task by ID with status derivation and enrichment.
   *
   * This method:
   * 1. Retrieves task from storage
   * 2. Derives status from last iteration
   * 3. Enriches with computed title
   * 4. Adds repository name
   *
   * @param taskId - Task ID to retrieve
   * @returns Enriched task with derived status
   * @throws TaskNotFoundError if task doesn't exist
   */
  async getTask(taskId: string): Promise<Task> {
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Derive task status from last iteration
    const taskWithStatus = deriveTaskStatus(task)

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

  /**
   * List tasks with optional filtering and pagination.
   *
   * By default:
   * - Excludes closed tasks (unless status filter explicitly includes them)
   * - Sorts by lastMessageAt (most recent first)
   * - Returns paginated results
   *
   * @param limit - Maximum number of tasks to return (default: 50)
   * @param offset - Number of tasks to skip (default: 0)
   * @param status - Optional status filter
   * @returns Object containing tasks array and total count
   */
  async listTasks(
    limit = 50,
    offset = 0,
    status?: string,
  ): Promise<{ tasks: Task[]; total: number }> {
    // If no status filter, exclude closed tasks by default
    const excludeClosed = !status

    // Use repository's efficient query
    const queryOptions: {
      limit: number
      offset: number
      status?: string
    } = {
      limit,
      offset,
    }

    if (status) {
      queryOptions.status = status
    }

    const result = await this.taskRepository.listTasks(queryOptions)

    // Filter out closed tasks if no status specified
    let filteredTasks = result.tasks
    if (excludeClosed) {
      filteredTasks = result.tasks.filter((task) => task.status !== 'closed')
    }

    // Apply status derivation to each task
    const tasksWithStatus = filteredTasks.map((task) => deriveTaskStatus(task))

    // Note: title and repositoryName are not needed for list view
    // They are computed in getTask() when full details are needed
    return {
      tasks: tasksWithStatus as Task[],
      total: excludeClosed ? tasksWithStatus.length : result.total,
    }
  }

  /**
   * Update task metadata (name and description) with validation.
   *
   * Task names must be URL-friendly (alphanumeric and dashes only).
   *
   * @param taskId - Task ID to update
   * @param updates - Name and/or description updates
   * @returns Updated task with enriched data
   * @throws TaskNotFoundError if task doesn't exist
   * @throws ValidationError if name format is invalid
   */
  async updateTaskMetadata(
    taskId: string,
    updates: { name?: string; description?: string },
  ): Promise<Task> {
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

    await this.taskRepository.save(task)
    return this.getTask(taskId)
  }

  /**
   * Update task with partial updates.
   *
   * This is a general-purpose update method that applies any partial updates
   * to the task. Use updateTaskMetadata for validated name/description updates.
   *
   * @param taskId - Task ID to update
   * @param updates - Partial task updates
   * @throws TaskNotFoundError if task doesn't exist
   */
  async updateTask(taskId: string, updates: Partial<Task>): Promise<void> {
    const task = await this.getTask(taskId)
    const updatedTask = { ...task, ...updates }
    await this.taskRepository.save(updatedTask)
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
    await this.executionCoordinator.createAndSetupWorkspace(task, params.repositoryId)

    logger.info('Task creation completed successfully', {
      taskId: task.taskId,
    })

    await this.executionCoordinator.handleTaskStartOrSetup(task, params)

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
    await this.previewOrchestrator.cleanup()
    await this.agentController.cleanup()
    await this.workspaceCoordinator.deleteSnapshot(taskId)

    // Delete task data
    await this.taskRepository.delete(taskId)
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
    await this.taskRepository.save(task)

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
    _contexts?: MessageContext[],
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
      await this.taskRepository.save(task)

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
}
