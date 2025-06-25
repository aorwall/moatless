import type { Task } from '../../shared/types/task.js'
import type { StorageBackend } from './storage.js'

export class TaskStorageService {
  constructor(private storage: StorageBackend) {}

  async saveTask(task: Task): Promise<void> {
    await this.storage.saveTask(task)
  }

  async getTask(taskId: string): Promise<Task | null> {
    return this.storage.getTask(taskId)
  }

  async listTasks(prefix?: string): Promise<string[]> {
    return this.storage.list(prefix || 'tasks/')
  }

  async deleteTask(taskId: string): Promise<void> {
    await this.storage.delete(`tasks/${taskId}`)
  }

  async deleteTaskEvents(taskId: string): Promise<void> {
    await this.storage.delete(`events/${taskId}`)
  }

  async getTaskWithEnvironmentId(
    taskId: string,
  ): Promise<{ task: Task; environmentId: string } | null> {
    const task = await this.getTask(taskId)
    if (!task) {
      return null
    }
    return {
      task,
      environmentId: task.environmentId,
    }
  }
}

export function createTaskStorageService(storage: StorageBackend): TaskStorageService {
  return new TaskStorageService(storage)
}
