import type { Task, TaskIteration } from '@moatless/types'
import type { SQL } from 'drizzle-orm'
import { and, desc, eq, exists, inArray } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import { assertOwnership } from '../../../features/auth'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol, withVisibilityScope } from '../../../infrastructure/database/database'
import { logger } from '../../../utils/logger'
import { tags, taskTags } from '../../tags/db/tag.schema'
import {
  iterationToInsert,
  selectToIteration,
  selectToTask,
  taskIterations,
  tasks,
  taskToInsert,
} from './task.schema'

/**
 * Repository for task persistence using Drizzle ORM.
 * Provides type-safe CRUD operations for tasks and iterations.
 */
@injectable()
export class TaskRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a task (insert or update on conflict).
   * Also saves all iterations.
   */
  async save(task: Task): Promise<void> {
    await this.db.transaction(async (tx) => {
      // Upsert task
      const taskInsert = taskToInsert(task)
      await tx.insert(tasks).values(taskInsert).onConflictDoUpdate({
        target: tasks.taskId,
        set: taskInsert,
      })

      // Delete existing iterations for this task
      await tx.delete(taskIterations).where(eq(taskIterations.taskId, task.taskId))

      // Insert all iterations
      if (task.iterations && task.iterations.length > 0) {
        const iterationInserts = task.iterations.map((iteration) =>
          iterationToInsert(task.taskId, iteration),
        )
        await tx.insert(taskIterations).values(iterationInserts)
      }
    })

    logger.debug({ taskId: task.taskId }, 'Task saved to database')
  }

  /**
   * Get a task by ID with all iterations.
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

    // Load iterations
    const iterationRows = await this.db.drizzle
      .select()
      .from(taskIterations)
      .where(eq(taskIterations.taskId, taskId))
      .orderBy(taskIterations.iterationNumber)

    return selectToTask(taskRow, iterationRows)
  }

  /**
   * Get a task by ID without access control.
   * Used for internal/system operations like MCP tool execution.
   * INTERNAL USE ONLY - bypasses user access control.
   *
   * @param taskId - Task ID to fetch
   * @returns Task or null if not found
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

    // Load iterations
    const iterationRows = await this.db.drizzle
      .select()
      .from(taskIterations)
      .where(eq(taskIterations.taskId, taskId))
      .orderBy(taskIterations.iterationNumber)

    return selectToTask(taskRow, iterationRows)
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
   * Get a task by short ID with all iterations.
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

    // Load iterations
    const iterationRows = await this.db.drizzle
      .select()
      .from(taskIterations)
      .where(eq(taskIterations.taskId, taskRow.taskId))
      .orderBy(taskIterations.iterationNumber)

    return selectToTask(taskRow, iterationRows)
  }

  /**
   * Delete a task and its iterations (cascade)
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
   * More efficient than loading all tasks and filtering in memory.
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

    // Load iterations for all tasks
    const taskIds = taskRows.map((row) => row.taskId)
    const allIterations =
      taskIds.length > 0
        ? await this.db.drizzle
            .select()
            .from(taskIterations)
            .where(inArray(taskIterations.taskId, taskIds))
            .orderBy(taskIterations.taskId, taskIterations.iterationNumber)
        : []

    // Group iterations by task
    const iterationsByTask = new Map<string, typeof allIterations>()
    for (const iteration of allIterations) {
      const existing = iterationsByTask.get(iteration.taskId) || []
      existing.push(iteration)
      iterationsByTask.set(iteration.taskId, existing)
    }

    // Convert to domain models
    const taskDomains = taskRows.map((taskRow) => {
      const iterations = iterationsByTask.get(taskRow.taskId) || []
      return selectToTask(taskRow, iterations)
    })

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

    // Load iterations for all tasks
    const taskIds = taskRows.map((row) => row.taskId)
    const allIterations =
      taskIds.length > 0
        ? await this.db.drizzle
            .select()
            .from(taskIterations)
            .where(inArray(taskIterations.taskId, taskIds))
            .orderBy(taskIterations.taskId, taskIterations.iterationNumber)
        : []

    // Group iterations by task
    const iterationsByTask = new Map<string, typeof allIterations>()
    for (const iteration of allIterations) {
      const existing = iterationsByTask.get(iteration.taskId) || []
      existing.push(iteration)
      iterationsByTask.set(iteration.taskId, existing)
    }

    // Convert to domain models
    const taskDomains = taskRows.map((taskRow) => {
      const iterations = iterationsByTask.get(taskRow.taskId) || []
      return selectToTask(taskRow, iterations)
    })

    return taskDomains
  }

  /**
   * Update task iterations.
   * Replaces all iterations for a task.
   */
  async updateIterations(taskId: string, iterations: TaskIteration[]): Promise<void> {
    await this.db.transaction(async (tx) => {
      // Delete existing iterations
      await tx.delete(taskIterations).where(eq(taskIterations.taskId, taskId))

      // Insert new iterations
      if (iterations.length > 0) {
        const iterationInserts = iterations.map((iteration) => iterationToInsert(taskId, iteration))
        await tx.insert(taskIterations).values(iterationInserts)
      }
    })

    logger.debug({ taskId, count: iterations.length }, 'Task iterations updated')
  }

  /**
   * Append a single iteration to a task.
   * More efficient than loading all iterations and saving.
   */
  async appendIteration(taskId: string, iteration: TaskIteration): Promise<void> {
    const iterationInsert = iterationToInsert(taskId, iteration)
    await this.db.drizzle.insert(taskIterations).values(iterationInsert)

    logger.debug({ taskId, iterationNumber: iteration.iterationNumber }, 'Iteration appended')
  }

  /**
   * Get all iterations for a task.
   * Useful when you only need iterations without full task data.
   */
  async getIterations(taskId: string): Promise<TaskIteration[]> {
    const rows = await this.db.drizzle
      .select()
      .from(taskIterations)
      .where(eq(taskIterations.taskId, taskId))
      .orderBy(taskIterations.iterationNumber)

    return rows.map(selectToIteration)
  }

  /**
   * Get a specific iteration.
   */
  async getIteration(taskId: string, iterationNumber: number): Promise<TaskIteration | null> {
    const rows = await this.db.drizzle
      .select()
      .from(taskIterations)
      .where(
        and(eq(taskIterations.taskId, taskId), eq(taskIterations.iterationNumber, iterationNumber)),
      )
      .limit(1)

    const iteration = rows[0]
    if (!iteration) {
      return null
    }

    return selectToIteration(iteration)
  }
}
