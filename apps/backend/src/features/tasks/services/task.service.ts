import { randomUUID } from 'node:crypto'
import type {
  AskUserQuestionResponse,
  Command,
  CreateTaskRequest,
  CreateVibeTaskRequest,
  ExitPlanModeResponse,
  MessageContext,
  RepositoryConfig,
  Tag,
  Task,
} from '@moatless/types'
import { RepositoryNotFoundError, TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { getCurrentUserIdForCreation, UserRepository } from '../../../features/auth'
import { generateShortId } from '../../../features/channels/utils/short-id.utils'
import { SettingsService } from '../../../services/settings.service'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { taskCreatedTotal, taskStatusTransitionsTotal } from '../../metrics'
import { RepositoryService } from '../../repositories/services/repository.service'
import { TagRepository } from '../../tags/db/tag.repository'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../db/task.repository'
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
 * - WorkspaceRuntimeManager - Workspace management (handles agent/preview cleanup)
 * - RepositoryService - Repository operations
 * - EventService - Event emission
 */
@injectable()
export class TaskService {
  constructor(
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(TaskExecutionCoordinator) private executionCoordinator: TaskExecutionCoordinator,
    @inject(TaskCommandGateway) private commandGateway: TaskCommandGateway,
    @inject(WorkspaceRuntimeManager) private workspaceCoordinator: WorkspaceRuntimeManager,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(EventService) private eventService: EventService,
    @inject(SettingsService) private settingsService: SettingsService,
    @inject(UserRepository) private userRepository: UserRepository,
    @inject(TagRepository) private tagRepository: TagRepository,
  ) {}

  // ========================================
  // Task Creation - Orchestration
  // ========================================

  /**
   * Create a new task with complete orchestration
   *
   * Workflow:
   * 1. Validate name format and repository
   * 2. Initialize task metadata
   * 3. Persist task and emit created event
   * 4. Delegate to coordinator for message handling and workspace setup:
   *    - If message provided: sendMessage() handles iteration, message, queue, workspace
   *    - If no message: ensureWorkspaceInBackground() just provisions workspace
   */
  async createTask(params: CreateTaskRequest): Promise<Task> {
    logger.info(
      {
        agentType: params.agentType,
        repositoryId: params.repositoryId,
        hasBranchName: Boolean(params.branchName),
        hasMessage: Boolean(params.message),
        hasFileIds: (params.fileIds?.length ?? 0) > 0,
        start: params.start ?? true,
        agentMode: params.agentMode,
        name: params.name,
        description: params.description,
      },
      'Create new task',
    )

    // Validate repository exists (fail fast before any task creation)
    const repository = await this.repositoryService.getRepositoryById(params.repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError(params.repositoryId)
    }

    // Validate name format if provided
    if (params.name && !/^[a-zA-Z0-9-]+$/.test(params.name)) {
      throw new ValidationError(
        'Task name must be URL-friendly (alphanumeric characters and dashes only)',
        { name: 'Invalid format' },
      )
    }

    // Validate visibility constraints (repository already fetched above)
    if (params.visibility === 'public' && repository.visibility !== 'public') {
      throw new ValidationError(
        'Cannot create public task on private repository. Task visibility must be private or repository must be public.',
        { visibility: 'Invalid for private repository' },
      )
    }

    const now = new Date()
    const task = await this.initializeTaskMetadata(params, now, repository)

    // Persist task FIRST (without iteration - coordinator will add it via sendMessage)
    await this.persistTaskAndEmitCreatedEvent(task, params, now)

    // Resolve tag names and associate with task
    if (params.tags && params.tags.length > 0) {
      for (const tagName of params.tags) {
        const tag = await this.tagRepository.getOrCreateByName(tagName)
        await this.tagRepository.addTagToTask(task.taskId, tag.tagId)
      }
    }

    taskCreatedTotal.labels({ agent_type: params.agentType }).inc()

    // Delegate to coordinator for unified execution flow
    if (params.message || (params.contexts && params.contexts.length > 0)) {
      // sendMessage handles: create iteration, save message, publish to queue, ensure workspace
      await this.executionCoordinator.sendMessage(
        task.taskId,
        params.message ?? '',
        false, // resume=false for initial task creation
        params.model,
        params.fileIds,
        undefined, // commandName
        params.agentMode,
        params.contexts,
      )
      logger.info(
        { taskId: task.taskId },
        'Task created, message published to queue, workspace provisioning in background',
      )
    } else {
      // No message - just ensure workspace starts provisioning
      this.executionCoordinator.ensureWorkspaceInBackground(task.taskId, params.repositoryId)
      logger.info(
        { taskId: task.taskId },
        'Task created (no message), workspace provisioning started in background',
      )
    }

    // Reload task to get updated state (with iteration if message was sent)
    return await this.getTask(task.taskId)
  }

  /**
   * Create a vibe task from template
   *
   * Workflow:
   * 1. Validate template exists
   * 2. Create task pointing directly to template repository
   *    (No new repository created - workspace will clone template in read-only mode)
   *
   * Template mode: The workspace clones the template repository but without
   * push credentials (pushEnabled: false in provision plan). Local git operations
   * work (commits for checkpointing), but push is disabled. "Publish" flow
   * (Phase 2) creates a new repository and enables push.
   */
  async createVibeTask(params: CreateVibeTaskRequest): Promise<string> {
    logger.debug(
      {
        agentType: params.agentType,
        templateId: params.templateId,
        hasMessage: Boolean(params.message),
        model: params.model,
        hasFileIds: (params.fileIds?.length ?? 0) > 0,
        start: params.start,
      },
      'Starting vibe task creation from template',
    )

    // 1. Validate template exists
    const template = await this.repositoryService.getRepositoryById(params.templateId)
    if (!template || template.kind !== 'template') {
      throw new ValidationError(`Template repository with id ${params.templateId} not found`, {
        templateId: 'Template repository not found',
      })
    }

    logger.info(
      { templateId: params.templateId, templateName: template.name },
      'Creating vibe task using template directly (template mode)',
    )

    // 2. Create task pointing directly to TEMPLATE repository
    const createTaskParams: CreateTaskRequest = {
      agentType: params.agentType,
      repositoryId: params.templateId, // Use template directly!
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

    // Verify agent is actually running when status shows 'running'
    // This corrects stale state when users reload/fetch tasks after missed completion events
    if (task.status === 'running') {
      const isActuallyRunning = await this.workspaceCoordinator.isAgentActuallyRunning(taskId)
      if (!isActuallyRunning) {
        logger.info(
          { taskId },
          'TaskService.getTask: Agent not running but task shows running - correcting stale state',
        )
        // Agent stopped but task still shows running — agent state is ephemeral
        // The stale task status will be corrected below when we set task.agentRunning
      }
    }

    // Fetch repository for display name
    const repository = await this.repositoryService.getRepositoryById(task.repositoryId)

    // Compute title from name, description, or agent type
    const title = task.name || task.description?.split('\n')[0] || task.agentType || 'Task'

    // Add repository name
    const repositoryName = repository?.name || task.repositoryId

    // Resolve createdBy user ID to display name
    const createdByName = await this.resolveUserDisplayName(task.createdBy)

    // Load tags for this task
    const taskTags = await this.tagRepository.getTagsForTask(taskId)

    return {
      ...task,
      title,
      repositoryName,
      createdByName,
      tags: taskTags.length > 0 ? taskTags : undefined,
    }
  }

  /**
   * Get task owner (createdBy) without access control.
   * Used for internal operations like MCP tool execution that need to establish
   * session context based on task ownership.
   * INTERNAL USE ONLY - bypasses user access control.
   *
   * @param taskId - Task ID
   * @returns User ID of task owner, or undefined if task has no owner
   * @throws {TaskNotFoundError} If task not found
   */
  async getTaskOwner(taskId: string): Promise<string | undefined> {
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    return task.createdBy
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
    filters?: { tag?: string; createdBy?: string; repositoryId?: string },
  ): Promise<{ tasks: Task[]; total: number }> {
    // If no status filter, exclude closed tasks by default
    const excludeClosed = !status

    // Use repository's efficient query
    const queryOptions: {
      limit: number
      offset: number
      status?: string
      tag?: string
      createdBy?: string
      repositoryId?: string
    } = {
      limit,
      offset,
    }

    if (status) {
      queryOptions.status = status
    }
    if (filters?.tag) {
      queryOptions.tag = filters.tag
    }
    if (filters?.createdBy) {
      queryOptions.createdBy = filters.createdBy
    }
    if (filters?.repositoryId) {
      queryOptions.repositoryId = filters.repositoryId
    }

    const result = await this.taskRepository.listTasks(queryOptions)

    // Filter out closed tasks if no status specified
    let filteredTasks = result.tasks
    if (excludeClosed) {
      filteredTasks = result.tasks.filter((task) => task.status !== 'closed')
    }

    // Batch-resolve createdBy user IDs to display names
    const uniqueUserIds = [
      ...new Set(filteredTasks.map((t) => t.createdBy).filter(Boolean)),
    ] as string[]
    const userNameMap = await this.resolveUserDisplayNames(uniqueUserIds)

    // Batch-load tags for all tasks
    const taskIds = filteredTasks.map((t) => t.taskId)
    const tagsMap = await this.tagRepository.getTagsForTasks(taskIds)

    // Batch-resolve repository IDs to display names
    const uniqueRepoIds = [
      ...new Set(filteredTasks.map((t) => t.repositoryId).filter(Boolean)),
    ] as string[]
    const repoNameMap = new Map<string, string>()
    await Promise.all(
      uniqueRepoIds.map(async (repoId) => {
        try {
          const repo = await this.repositoryService.getRepositoryById(repoId)
          if (repo?.name) {
            repoNameMap.set(repoId, repo.name)
          }
        } catch {
          // Ignore missing repositories
        }
      }),
    )

    const tasksWithNames = filteredTasks.map((task) => {
      const taskTagsList = tagsMap.get(task.taskId)
      return {
        ...task,
        createdByName: task.createdBy ? userNameMap.get(task.createdBy) : undefined,
        repositoryName: task.repositoryId
          ? (repoNameMap.get(task.repositoryId) ?? task.repositoryId)
          : undefined,
        tags: taskTagsList && taskTagsList.length > 0 ? taskTagsList : undefined,
      }
    })

    return {
      tasks: tasksWithNames as Task[],
      total: excludeClosed ? tasksWithNames.length : result.total,
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
   * Update task metadata with repository renaming (for vibe tasks) and event emission
   *
   * This is the high-level method that should be used by tools and API endpoints
   * to update task metadata. It handles:
   * - Vibe repository renaming
   * - Task metadata updates
   * - Event emission for real-time updates
   *
   * @param taskId - The task ID to update
   * @param updates - The metadata updates to apply
   * @returns The updated task
   * @throws ValidationError if name format is invalid
   */
  async updateTaskWithEvent(
    taskId: string,
    updates: { name?: string; description?: string },
  ): Promise<Task> {
    // Use getInternal to bypass access control since this runs in a background
    // event handler context without user session
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Validate name format if provided
    if (updates.name !== undefined && updates.name && !/^[a-zA-Z0-9-]+$/.test(updates.name)) {
      throw new ValidationError(
        'Task name must be URL-friendly (alphanumeric characters and dashes only)',
        { name: 'Invalid format' },
      )
    }

    // Update only the provided fields (use internal access since this runs
    // in a background event handler context without user session)
    if (updates.name !== undefined) {
      task.name = updates.name || undefined
    }
    if (updates.description !== undefined) {
      task.description = updates.description || undefined
    }
    await this.taskRepository.save(task)

    // Get the updated task for the event payload (use getInternal for consistency)
    const updatedTask = await this.taskRepository.getInternal(taskId)
    if (!updatedTask) {
      throw new TaskNotFoundError(taskId)
    }

    // Emit task updated event
    await this.eventService.emitEvent({
      taskId,
      type: 'task',
      subType: 'updated',
      agentType: updatedTask.agentType,
      payload: {
        status: updatedTask.status,
        previousStatus: updatedTask.status,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: false,
    })

    return updatedTask
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

  /**
   * Replace all tags on a task with the given tag names.
   * Each tag name is get-or-created. Returns the resolved tag objects.
   */
  async setTaskTags(taskId: string, tagNames: string[]): Promise<Tag[]> {
    await this.getTask(taskId)

    const resolvedTags: Tag[] = []
    for (const name of tagNames) {
      const tag = await this.tagRepository.getOrCreateByName(name)
      resolvedTags.push(tag)
    }

    await this.tagRepository.setTagsForTask(
      taskId,
      resolvedTags.map((t) => t.tagId),
    )

    return resolvedTags
  }

  // ========================================
  // Task Execution - Delegation Methods
  // ========================================

  /**
   * Start a task that has a 'created' iteration.
   * Delegates to TaskExecutionCoordinator.
   *
   * @param taskId - Task ID to start
   */
  async startTask(taskId: string): Promise<void> {
    return await this.executionCoordinator.startTask(taskId)
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
    toolUseId?: string,
    toolName?: 'AskUserQuestion' | 'ExitPlanMode',
    toolResponse?: AskUserQuestionResponse | ExitPlanModeResponse,
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
      toolUseId,
      toolName,
      toolResponse,
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
  // Task Deletion - Orchestration
  // ========================================

  /**
   * Delete a task completely
   *
   * Workflow:
   * 1. Verify task exists
   * 2. Clean up workspace (stops agents, servers, artifacts)
   * 3. Delete task storage
   * 4. Delete event storage
   */
  async deleteTask(taskId: string): Promise<void> {
    await this.getTask(taskId)

    // Clean up workspace (stops agents, servers, containers, etc.)
    await this.workspaceCoordinator.deleteWorkspace(taskId)

    // Delete task data
    await this.taskRepository.delete(taskId)
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

    logger.debug({ taskId }, 'Closing task')

    // Delete workspace snapshot
    await this.workspaceCoordinator.deleteWorkspace(taskId)

    // Update task status to closed
    const previousStatus = task.status
    task.status = 'closed'
    await this.taskRepository.save(task)

    // Track task status transition metric
    taskStatusTransitionsTotal.labels({ status: 'closed', agent_type: task.agentType }).inc()

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

    logger.info({ taskId }, 'Task closed successfully')
  }

  // ========================================
  // Private Helper Methods
  // ========================================

  /**
   * Resolve a single user ID to a display name.
   * Returns login (preferred) or name, or undefined if user not found.
   */
  private async resolveUserDisplayName(userId: string | undefined): Promise<string | undefined> {
    if (!userId) {
      return undefined
    }
    try {
      const user = await this.userRepository.getUser(userId)
      return user?.login ?? user?.name ?? undefined
    } catch {
      return undefined
    }
  }

  /**
   * Batch-resolve user IDs to display names.
   * Returns a Map from userId to display name.
   */
  private async resolveUserDisplayNames(userIds: string[]): Promise<Map<string, string>> {
    const nameMap = new Map<string, string>()
    await Promise.all(
      userIds.map(async (userId) => {
        const name = await this.resolveUserDisplayName(userId)
        if (name) {
          nameMap.set(userId, name)
        }
      }),
    )
    return nameMap
  }

  /**
   * Initialize task metadata object with all required fields
   */
  private async initializeTaskMetadata(
    params: CreateTaskRequest,
    now: Date,
    repository: RepositoryConfig,
  ): Promise<Task> {
    // Determine visibility: use provided value, or use env var default (only if repo is public)
    // Tasks cannot be public if the repository is private
    const defaultTaskVisibility = this.settingsService.getDefaultTaskVisibility()
    const visibility: 'private' | 'public' =
      params.visibility ?? (repository.visibility === 'public' ? defaultTaskVisibility : 'private')

    const taskId = randomUUID()
    const shortId = await this.generateUniqueShortId()

    return {
      taskId,
      shortId,
      createdAt: now,
      lastMessageAt: now,
      agentType: params.agentType,
      repositoryId: params.repositoryId,
      status: 'open',
      totalCostUsd: 0.0,
      templateId: params.templateId,
      vibe: params.templateId ? true : undefined,
      name: params.name,
      description: params.description,
      branch: params.branch || params.branchName, // Branch to clone from
      createdBy: getCurrentUserIdForCreation(),
      visibility,
      siteUrl: undefined,
    }
  }

  /**
   * Generate a unique short ID for a task.
   * Retries up to 10 times if collision detected.
   */
  private async generateUniqueShortId(): Promise<string> {
    const maxAttempts = 10

    for (let i = 0; i < maxAttempts; i++) {
      const shortId = generateShortId()
      const exists = await this.taskRepository.existsByShortId(shortId)
      if (!exists) {
        return shortId
      }
      logger.debug({ shortId, attempt: i + 1 }, 'Short ID collision, retrying')
    }

    throw new Error('Failed to generate unique short ID after 10 attempts')
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

      logger.debug(
        {
          taskId: task.taskId,
          agentType: params.agentType,
          repositoryId: params.repositoryId,
          hasInitialIteration: Boolean(params.message),
        },
        'Task created successfully',
      )
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to save task metadata')
      throw error
    }
  }
}
