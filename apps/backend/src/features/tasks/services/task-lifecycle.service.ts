import type { Task } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { RepositoryService } from '../../repositories/index'
import { TaskMetadataStorage } from '../storage/task-metadata.storage'
import { deriveTaskStatus } from '../utils/task-status.utils'

/**
 * TaskLifecycleService - Responsible for task CRUD operations
 *
 * Handles:
 * - Reading task metadata with status derivation and enrichment
 * - Listing tasks with filtering and pagination
 * - Updating task metadata with validation
 * - General task updates
 *
 * Does NOT handle:
 * - Task creation (complex orchestration - belongs in TaskFacade/Coordinator)
 * - Task deletion (requires workspace cleanup - belongs in Coordinator)
 * - Task execution (belongs in TaskExecutionCoordinator)
 * - Iteration management (belongs in TaskIterationService)
 *
 * @see docs/patterns/backend-feature-architecture.md
 */
@injectable()
export class TaskLifecycleService {
  constructor(
    @inject(TaskMetadataStorage) private readonly taskStorage: TaskMetadataStorage,
    @inject(RepositoryService) private readonly repositoryService: RepositoryService,
  ) {}

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
    const task = await this.taskStorage.get(taskId)
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
    const taskKeys = await this.taskStorage.listTaskKeys()
    const tasks: Task[] = []

    for (const key of taskKeys) {
      const taskId = key.replace('tasks/', '')
      const task = await this.taskStorage.get(taskId)
      if (task) {
        // Apply status derivation
        const taskWithDerivedStatus = deriveTaskStatus(task)

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
   * Update task metadata (name and description) with validation.
   *
   * Task names must be URL-friendly (alphanumeric and dashes only).
   *
   * @param taskId - Task ID to update
   * @param updates - Name and/or description updates
   * @throws TaskNotFoundError if task doesn't exist
   * @throws ValidationError if name format is invalid
   */
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

    await this.taskStorage.save(task)
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
    await this.taskStorage.save(updatedTask)
  }
}
