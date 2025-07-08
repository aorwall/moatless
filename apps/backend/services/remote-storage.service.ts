import type { Event, Task, TaskIteration, Workspace } from '@moatless/shared/types/index'
import type { RepositoryConfig } from '@moatless/shared/types/repository'
import type { StorageBackend } from './storage.js'

export class RemoteStorageBackend implements StorageBackend {
  // private logger = createLogger()

  constructor(
    private apiUrl: string,
    private apiKey: string,
  ) {}

  private async request(path: string, options: RequestInit = {}): Promise<any> {
    const response = await fetch(`${this.apiUrl}/api/v1${path}`, {
      ...options,
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${this.apiKey}`,
        ...options.headers,
      },
    })

    if (!response.ok) {
      throw new Error(`Storage request failed: ${response.statusText}`)
    }

    return response.json()
  }

  async getTask(taskId: string): Promise<Task | null> {
    try {
      return await this.request(`/tasks/${taskId}`)
    } catch {
      return null
    }
  }

  async saveTask(task: Task): Promise<void> {
    await this.request(`/tasks/${task.taskId}`, {
      method: 'PUT',
      body: JSON.stringify(task),
    })
  }

  async updateIteration(taskId: string, iteration: TaskIteration): Promise<void> {
    await this.request(`/tasks/${taskId}/iterations/${iteration.iterationNumber}`, {
      method: 'PUT',
      body: JSON.stringify(iteration),
    })
  }

  async getRepository(repositoryId: string): Promise<RepositoryConfig | null> {
    try {
      return await this.request(`/repositories/${repositoryId}`)
    } catch {
      return null
    }
  }

  async saveRepository(repo: RepositoryConfig): Promise<void> {
    await this.request(`/repositories/${repo.id}`, {
      method: 'PUT',
      body: JSON.stringify(repo),
    })
  }

  async saveWorkspace(workspace: Workspace): Promise<void> {
    await this.request(`/workspaces/${workspace.id}`, {
      method: 'PUT',
      body: JSON.stringify(workspace),
    })
  }

  async getWorkspace(taskId: string): Promise<Workspace | null> {
    try {
      return await this.request(`/workspaces/${taskId}`)
    } catch {
      return null
    }
  }

  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.request(`/tasks/${taskId}/events`, {
      method: 'PUT',
      body: JSON.stringify(events),
    })
  }

  async getEvents(taskId: string): Promise<Event[]> {
    return this.request(`/tasks/${taskId}/events`)
  }

  // Generic storage methods - not needed in worker mode
  async save(_key: string, _data: unknown): Promise<void> {
    throw new Error('Generic save not available in worker mode')
  }

  async get<T = unknown>(_key: string): Promise<T | null> {
    throw new Error('Generic get not available in worker mode')
  }

  async list(_prefix?: string): Promise<string[]> {
    throw new Error('List not available in worker mode')
  }

  async delete(_key: string): Promise<void> {
    throw new Error('Delete not available in worker mode')
  }

  async exists(_key: string): Promise<boolean> {
    throw new Error('Exists not available in worker mode')
  }
}
