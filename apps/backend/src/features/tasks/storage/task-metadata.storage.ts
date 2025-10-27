import type { Task } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Storage } from '../../../infrastructure/storage/storage'
import { InfrastructureStorageSymbol } from '../../../infrastructure/storage/storage'

/**
 * Storage adapter for task metadata persistence.
 * Uses the infrastructure Storage interface to persist tasks as JSON.
 * Tasks are stored at key: tasks/{taskId}
 */
@injectable()
export class TaskMetadataStorage {
  constructor(@inject(InfrastructureStorageSymbol) private readonly storage: Storage) {}

  /**
   * Save a task to storage
   * @param task - Task object to persist
   */
  async save(task: Task): Promise<void> {
    await this.storage.save(this.getKey(task.taskId), task)
  }

  /**
   * Get a task by ID
   * @param taskId - Task identifier
   * @returns Task object or null if not found
   */
  async get(taskId: string): Promise<Task | null> {
    return await this.storage.get<Task>(this.getKey(taskId))
  }

  /**
   * Delete a task and its associated events
   * @param taskId - Task identifier
   */
  async delete(taskId: string): Promise<void> {
    await this.storage.delete(this.getKey(taskId))
    // Also delete associated events
    await this.storage.delete(`events/${taskId}`)
  }

  /**
   * Check if a task exists
   * @param taskId - Task identifier
   * @returns True if task exists
   */
  async exists(taskId: string): Promise<boolean> {
    return await this.storage.exists(this.getKey(taskId))
  }

  /**
   * List all task keys
   * @returns Array of task keys (e.g., ['tasks/task-123', 'tasks/task-456'])
   */
  async listTaskKeys(): Promise<string[]> {
    return await this.storage.list('tasks/')
  }

  /**
   * Generate storage key for a task
   * @param taskId - Task identifier
   * @returns Storage key
   */
  private getKey(taskId: string): string {
    return `tasks/${taskId}`
  }
}
