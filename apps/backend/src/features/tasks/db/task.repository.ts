import type { Task } from '@moatless/types'
import type { SQL } from 'drizzle-orm'
import { and, desc, eq, exists } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import { assertOwnership } from '../../../features/auth'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol, withVisibilityScope } from '../../../infrastructure/database/database'
import { logger } from '../../../utils/logger'
import { tags, taskTags } from '../../tags/db/tag.schema'
import { selectToTask, tasks, taskToInsert } from './task.schema'

/**
 * Repository for task persistence using Drizzle ORM.
 * Provides type-safe CRUD operations for tasks.
 */
@injectable()
export class TaskRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a task (insert or update on conflict).
   */
  async save(task: Task): Promise<void> {
    const taskInsert = taskToInsert(task)
    await this.db.drizzle.insert(tasks).values(taskInsert).onConflictDoUpdate({
      target: tasks.taskId,
      set: taskInsert,
    })

    logger.debug({ taskId: task.taskId }, 'Task saved to database')
  }

  /**
   * Get a task by ID.
   * Returns null if task not found.
   * Applies user-scoped access control.
   */
  async get(taskId: string): Promise<Omit<Task, 'title' | 'repositoryName'> | null> {
    const taskRows = await this.db.drizzle
      .select()
      .from(tasks)
      .where(
        withVisibilityScope(tasks, {
          additionalConditions: [eq(tasks.taskId, taskId)],
        }),
      )
      .limit(1)

    const taskRow = taskRows[0]
    if (!taskRow) {
      return null
    }

    return selectToTask(taskRow)
  }

  /**
   * Get a task by ID without access control.
   * Used for internal/system operations like MCP tool execution.
   * INTERNAL USE ONLY - bypasses user access control.
   */
  async getInternal(taskId: string): Promise<Omit<Task, 'title' | 'repositoryName'> | null> {
    const taskRows = await this.db.drizzle
      .select()
      .from(tasks)
      .where(eq(tasks.taskId, taskId))
      .limit(1)

    const taskRow = taskRows[0]
    if (!taskRow) {
      return null
    }

    return selectToTask(taskRow)
  }

  /**
   * Check if a task exists
   */
  async exists(taskId: string): Promise<boolean> {
    const result = await this.db.drizzle
      .select({ taskId: tasks.taskId })
      .from(tasks)
      .where(eq(tasks.taskId, taskId))
      .limit(1)

    return result.length > 0
  }

  /**
   * Check if a shortId is already in use
   */
  async existsByShortId(shortId: string): Promise<boolean> {
    const result = await this.db.drizzle
      .select({ shortId: tasks.shortId })
      .from(tasks)
      .where(eq(tasks.shortId, shortId))
      .limit(1)

    return result.length > 0
  }

  /**
   * Get a task by short ID.
   * Returns null if task not found.
   * Applies user-scoped access control.
   */
  async getByShortId(shortId: string): Promise<Omit<Task, 'title' | 'repositoryName'> | null> {
    const taskRows = await this.db.drizzle
      .select()
      .from(tasks)
      .where(
        withVisibilityScope(tasks, {
          additionalConditions: [eq(tasks.shortId, shortId)],
        }),
      )
      .limit(1)

    const taskRow = taskRows[0]
    if (!taskRow) {
      return null
    }

    return selectToTask(taskRow)
  }

  /**
   * Delete a task (cascade deletes related data).
   * Verifies user ownership before deletion.
   * @throws {ForbiddenError} If user doesn't own the task
   */
  async delete(taskId: string): Promise<void> {
    // Get task first to verify ownership
    const task = await this.get(taskId)
    if (!task) {
      // Task doesn't exist or user doesn't have access
      return
    }

    // Verify ownership (throws ForbiddenError if not authorized)
    assertOwnership(task)

    await this.db.drizzle.delete(tasks).where(eq(tasks.taskId, taskId))
    logger.debug({ taskId }, 'Task deleted from database')
  }

  /**
   * List all task IDs.
   * Returns array of task IDs in format expected by legacy code.
   */
  async listTaskKeys(): Promise<string[]> {
    const result = await this.db.drizzle.select({ taskId: tasks.taskId }).from(tasks)

    // Return in format expected by TaskService: 'tasks/{taskId}'
    return result.map((row) => `tasks/${row.taskId}`)
  }

  /**
   * List tasks with filtering, sorting, and pagination.
   * Applies user-scoped access control.
   */
  async listTasks(options: {
    limit?: number
    offset?: number
    status?: string
    repositoryId?: string
    tag?: string
    createdBy?: string
  }): Promise<{
    tasks: Omit<Task, 'title' | 'repositoryName'>[]
    total: number
  }> {
    const { limit = 50, offset = 0, status, repositoryId, tag, createdBy } = options

    // Build WHERE conditions
    const conditions: SQL[] = []
    if (status) {
      conditions.push(eq(tasks.status, status))
    }
    if (repositoryId) {
      conditions.push(eq(tasks.repositoryId, repositoryId))
    }
    if (createdBy) {
      conditions.push(eq(tasks.createdBy, createdBy))
    }
    if (tag) {
      conditions.push(
        exists(
          this.db.drizzle
            .select({ one: taskTags.taskId })
            .from(taskTags)
            .innerJoin(tags, eq(taskTags.tagId, tags.tagId))
            .where(and(eq(taskTags.taskId, tasks.taskId), eq(tags.name, tag))),
        ),
      )
    }

    // Apply visibility-scoped access control
    const whereClause = withVisibilityScope(tasks, {
      additionalConditions: conditions.length > 0 ? conditions : [],
    })

    // Get total count
    const countResult = await this.db.drizzle
      .select({ count: tasks.id })
      .from(tasks)
      .where(whereClause)

    const total = countResult.length

    // Get paginated tasks
    const taskRows = await this.db.drizzle
      .select()
      .from(tasks)
      .where(whereClause)
      .orderBy(desc(tasks.lastMessageAt))
      .limit(limit)
      .offset(offset)

    // Convert to domain models
    const taskDomains = taskRows.map((taskRow) => selectToTask(taskRow))

    return { tasks: taskDomains, total }
  }

  /**
   * List all tasks without access control.
   * Used for system-level operations like startup recovery and graceful shutdown.
   * INTERNAL USE ONLY - bypasses user access control.
   */
  async listAll(): Promise<Omit<Task, 'title' | 'repositoryName'>[]> {
    // Get all tasks without access control
    const taskRows = await this.db.drizzle.select().from(tasks).orderBy(desc(tasks.lastMessageAt))

    // Convert to domain models
    return taskRows.map((taskRow) => selectToTask(taskRow))
  }
}
