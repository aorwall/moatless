import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { FastifyInstance } from 'fastify'
import type { Environment, Event, TaskMetadata, Workspace } from '../../shared/types/index.js'
import { fromJSON, toJSON } from '../../shared/utils/validation.js'

export interface StorageBackend {
  // Generic storage methods
  save(key: string, data: any): Promise<void>
  get<T = any>(key: string): Promise<T | null>
  list(prefix?: string): Promise<string[]>
  delete(key: string): Promise<void>
  exists(key: string): Promise<boolean>

  // Type-safe convenience methods
  saveTask(task: TaskMetadata): Promise<void>
  getTask(taskId: string): Promise<TaskMetadata | null>
  saveEnvironment(env: Environment): Promise<void>
  getEnvironment(envId: string): Promise<Environment | null>
  saveWorkspace(workspace: Workspace): Promise<void>
  getWorkspace(taskId: string): Promise<Workspace | null>
  saveEvents(taskId: string, events: Event[]): Promise<void>
  getEvents(taskId: string): Promise<Event[]>
}

export class FileStorage implements StorageBackend {
  constructor(private basePath: string) {}

  async save(key: string, data: any): Promise<void> {
    const filePath = await this.getPath(key)
    const dir = path.dirname(filePath)
    await fs.mkdir(dir, { recursive: true })
    await fs.writeFile(filePath, toJSON(data))
  }

  async get<T = any>(key: string): Promise<T | null> {
    try {
      const filePath = await this.getPath(key)
      const content = await fs.readFile(filePath, 'utf-8')
      return fromJSON<T>(content)
    } catch {
      return null
    }
  }

  async list(prefix?: string): Promise<string[]> {
    const dir = path.join(this.basePath, prefix || '')
    try {
      const entries = await fs.readdir(dir, { withFileTypes: true })
      const files: string[] = []

      for (const entry of entries) {
        if (entry.isFile() && entry.name.endsWith('.json')) {
          const key = prefix
            ? `${prefix}${entry.name.replace('.json', '')}`
            : entry.name.replace('.json', '')
          files.push(key)
        } else if (entry.isDirectory() && prefix) {
          // Recursively list subdirectories
          const subFiles = await this.list(`${prefix}${entry.name}/`)
          files.push(...subFiles)
        }
      }

      return files.sort()
    } catch {
      return []
    }
  }

  async delete(key: string): Promise<void> {
    const filePath = await this.getPath(key)
    await fs.unlink(filePath)
  }

  async exists(key: string): Promise<boolean> {
    const filePath = await this.getPath(key)
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  // Type-safe convenience methods
  async saveTask(task: TaskMetadata): Promise<void> {
    await this.save(`tasks/${task.taskId}`, task)
  }

  async getTask(taskId: string): Promise<TaskMetadata | null> {
    return this.get<TaskMetadata>(`tasks/${taskId}`)
  }

  async saveEnvironment(env: Environment): Promise<void> {
    await this.save(`environments/${env.id}`, env)
  }

  async getEnvironment(envId: string): Promise<Environment | null> {
    return this.get<Environment>(`environments/${envId}`)
  }

  async saveWorkspace(workspace: Workspace): Promise<void> {
    await this.save(`workspaces/${workspace.id}`, workspace)
  }

  async getWorkspace(taskId: string): Promise<Workspace | null> {
    return this.get<Workspace>(`workspaces/${taskId}`)
  }

  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.save(`events/${taskId}`, events)
  }

  async getEvents(taskId: string): Promise<Event[]> {
    const events = await this.get<Event[]>(`events/${taskId}`)
    return events || []
  }

  private async getPath(key: string): Promise<string> {
    return path.join(this.basePath, `${key}.json`)
  }
}

export function createStorageService(_app: FastifyInstance): StorageBackend {
  const dataDir = process.env.DATA_DIR || './data'
  return new FileStorage(dataDir)
}
