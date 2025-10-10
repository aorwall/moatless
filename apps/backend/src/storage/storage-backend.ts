import type { Event, RepositoryConfig, Task, User, Workspace } from '@moatless/types'

export interface StorageBackend {
  // Generic storage methods
  save(key: string, data: unknown): Promise<void>
  get<T = unknown>(key: string): Promise<T | null>
  list(prefix?: string): Promise<string[]>
  delete(key: string): Promise<void>
  exists(key: string): Promise<boolean>

  // JSONL methods for append-only data
  appendJsonl(key: string, data: unknown): Promise<void>
  readJsonl<T = unknown>(key: string): Promise<T[]>

  // Type-safe convenience methods
  saveTask(task: Task): Promise<void>
  getTask(taskId: string): Promise<Task | null>
  saveRepository(repo: RepositoryConfig): Promise<void>
  getRepository(repoId: string): Promise<RepositoryConfig | null>
  saveWorkspace(workspace: Workspace): Promise<void>
  getWorkspace(taskId: string): Promise<Workspace | null>
  saveEvents(taskId: string, events: Event[]): Promise<void>
  getEvents(taskId: string): Promise<Event[]>
  saveUser(user: User): Promise<void>
  getUser(userId: number): Promise<User | null>
  listUsers(): Promise<User[]>
}
